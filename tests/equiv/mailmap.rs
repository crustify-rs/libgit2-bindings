//! Equivalence of `git2/mailmap.h`'s [`Mailmap`] constructors with the C routines they
//! wrap: `git_mailmap_new`, `git_mailmap_from_buffer` and
//! `git_mailmap_from_repository`, released by `git_mailmap_free`.
//!
//! Each mailmap is built twice, independently, one per side, from equal inputs: C
//! calls the routine; Rust calls the constructor and reaches its mailmap through the
//! box's `as_c_ptr`. The lookups have no wrappers yet, so both sides then drive their
//! object through the same raw calls: `git_mailmap_resolve` over a fixed set of probe
//! identities, then a sequence of `git_mailmap_add_entry` calls (adding entries,
//! replacing one, adding a name-specific entry beside a nameless one), resolving the
//! probes again after each step. Every step records the return code, the resolved name
//! and email, and for a failure the error libgit2 left (class and message); the
//! records are compared step by step. Each side then frees its own mailmap.
//!
//! Two documented divergences, both in [`Mailmap::from_buffer`]:
//!
//! - a buffer with a NUL byte fails on both sides, but libgit2 sets no error, while the wrapper
//!   reports `GIT_ERROR_INVALID`, "mailmap buffer contains a NUL byte";
//! - a line whose replaced email is empty (`Name <>`) makes libgit2 fail the whole buffer with a
//!   failed argument assertion and leak the entry it began; the wrapper skips the line, as libgit2
//!   skips every other line that is not an entry. C is not run on such lines here (the leak fails
//!   `LeakSanitizer`); the Rust result is compared with C's on the same buffer without them, and
//!   the classification itself with C's refusals in the crate's unit tests.

use std::ffi::{CStr, CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::{fs, ptr};

use libgit2::ffi;
use libgit2::libgit2::object::mailmap::{Mailmap, MailmapBox};
use libgit2::libgit2::repository::repository::Repository;
use libgit2::libgit2::util::errors::{ErrorClass, last};

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

/// One step's observation: the return code, the resolved `(name, email)` when one was
/// read, and for a failure the error libgit2 left, `(class, message)`.
type Record = (c_int, Option<(String, String)>, Option<(c_int, String)>);

/// The calling thread's last error, read through C.
fn c_error() -> (c_int, String) {
    // SAFETY: no arguments; libgit2 is initialised, so it returns a live error.
    let error = unsafe { ffi::git_error_last() };
    // SAFETY: as above, non-NULL and live.
    let error = unsafe { &*error };
    // SAFETY: a libgit2 error's message is NUL-terminated, live until the next call.
    let message = unsafe { CStr::from_ptr(error.message) };
    (error.klass, message.to_string_lossy().into_owned())
}

/// libgit2's own mailmap test file (`tests/resources/mailmap/.mailmap`).
const MAILMAP: &[u8] = b"\
# Simple Comment line
<cto@company.xx>                       <cto@coompany.xx>
Some Dude <some@dude.xx>         nick1 <bugs@company.xx>
Other Author <other@author.xx>   nick2 <bugs@company.xx>
Other Author <other@author.xx>         <nick2@company.xx>
Phil Hill <phil@company.xx>  # Comment at end of line
<joseph@company.xx>             Joseph <bugs@company.xx>
Santa Claus <santa.claus@northpole.xx> <me@company.xx>
Untracked <untracked@company.xx>
";

/// The identities resolved after every step: the reference file's, the generated
/// buffers', the added entries', and misses.
fn probes() -> Vec<(CString, CString)> {
    let names = [
        "Brad", "nick1", "nick2", "nick3", "Joseph", "Name", "A", "", "xx",
    ];
    let emails = [
        "cto@coompany.xx",
        "bugs@company.xx",
        "nick2@company.xx",
        "phil@company.xx",
        "me@company.xx",
        "untracked@company.xx",
        "a@x",
        "b@x",
        "",
        "nobody@x",
        "fileoverridename@company.xx",
    ];
    let mut out = Vec::new();
    for name in names {
        for email in emails {
            out.push((CString::new(name).unwrap(), CString::new(email).unwrap()));
        }
    }
    out
}

/// `git_mailmap_resolve` of every probe on `mailmap`.
fn resolve_all(mailmap: *const ffi::git_mailmap) -> Vec<Record> {
    probes()
        .iter()
        .map(|(name, email)| {
            let (mut real_name, mut real_email): (*const c_char, *const c_char) =
                (ptr::null(), ptr::null());
            // SAFETY: two writable slots; `mailmap` is live for the call, which only
            // reads it; `name` and `email` are NUL-terminated.
            let rc = unsafe {
                ffi::git_mailmap_resolve(
                    &raw mut real_name,
                    &raw mut real_email,
                    mailmap,
                    name.as_ptr(),
                    email.as_ptr(),
                )
            };
            if rc < 0 {
                return (rc, None, Some(c_error()));
            }
            // SAFETY: on success both point at `name`, `email` or an entry's strings,
            // NUL-terminated and live until `mailmap` or the probe is released, after
            // this copy.
            let cstr = unsafe { CStr::from_ptr(real_name) };
            // SAFETY: as above.
            let resolved = unsafe {
                (
                    cstr.to_string_lossy().into_owned(),
                    CStr::from_ptr(real_email).to_string_lossy().into_owned(),
                )
            };
            (rc, Some(resolved), None)
        })
        .collect()
}

/// How many probes the first resolution of `steps` maps to another identity: whether
/// the compared mailmap holds entries at all.
fn mapped(steps: &[Vec<Record>]) -> usize {
    steps[0]
        .iter()
        .zip(probes())
        .filter(|((_, resolved, _), (name, email))| {
            resolved.as_ref().is_some_and(|(n, e)| {
                (n.as_bytes(), e.as_bytes()) != (name.to_bytes(), email.to_bytes())
            })
        })
        .count()
}

/// The `git_mailmap_add_entry` steps, each followed by a full resolution.
fn add_steps(mailmap: *mut ffi::git_mailmap) -> Vec<Vec<Record>> {
    type Entry = (
        Option<&'static CStr>,
        Option<&'static CStr>,
        Option<&'static CStr>,
        &'static CStr,
    );
    let entries: [Entry; 6] = [
        (Some(c"Real A"), Some(c"real.a@x"), None, c"a@x"),
        (Some(c"Named A"), None, Some(c"A"), c"a@x"),
        (None, Some(c"fixed@x"), Some(c"Name"), c"b@x"),
        (Some(c"Replaced A"), Some(c"replaced@x"), None, c"a@x"),
        (Some(c"Brad Real"), None, Some(c"Brad"), c"cto@coompany.xx"),
        (None, None, None, c"nobody@x"),
    ];
    let opt = |s: Option<&CStr>| s.map_or(ptr::null(), CStr::as_ptr);
    let mut steps = vec![resolve_all(mailmap)];
    for (real_name, real_email, replace_name, replace_email) in entries {
        // SAFETY: `mailmap` is live and used by this thread alone; each string is NULL
        // or NUL-terminated, and `replace_email` non-empty, which libgit2 requires.
        let rc = unsafe {
            ffi::git_mailmap_add_entry(
                mailmap,
                opt(real_name),
                opt(real_email),
                opt(replace_name),
                replace_email.as_ptr(),
            )
        };
        let mut step = vec![(rc, None, (rc < 0).then(c_error))];
        step.extend(resolve_all(mailmap));
        steps.push(step);
    }
    steps
}

/// The C side of `from_buffer`: the return code, the error it left, and the steps on
/// the mailmap it built (none on failure). Frees what it built.
fn c_from_buffer(buf: &[u8]) -> (c_int, Option<(c_int, String)>, Vec<Vec<Record>>) {
    let mut out = ptr::null_mut();
    // SAFETY: no arguments.
    unsafe { ffi::git_error_clear() };
    let data: *const c_char = if buf.is_empty() {
        c"".as_ptr()
    } else {
        buf.as_ptr().cast()
    };
    // SAFETY: a writable slot; `data` addresses `buf.len()` readable bytes (one, for an
    // empty buffer).
    let rc = unsafe { ffi::git_mailmap_from_buffer(&raw mut out, data, buf.len()) };
    if rc < 0 {
        assert!(out.is_null());
        return (rc, Some(c_error()), Vec::new());
    }
    let steps = add_steps(out);
    // SAFETY: the mailmap C built, freed once.
    unsafe { ffi::git_mailmap_free(out) };
    (rc, None, steps)
}

/// The steps on a Rust-built mailmap, which is then dropped.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the helper consumes what it reports, as its callers hand it over"
)]
fn rust_steps(mailmap: MailmapBox) -> Vec<Vec<Record>> {
    add_steps(mailmap.as_c_ptr())
}

/// Lines from the tokens the parser tells apart, grouped deterministically into small
/// buffers, keeping those that spell no empty email.
fn generated_buffers() -> Vec<Vec<u8>> {
    const TOKENS: [&[u8]; 9] = [
        b" ", b"\t", b"Name ", b"<a@x>", b"<b@x>", b"# c", b"<a#b>", b"<", b">",
    ];
    let mut lines = Vec::new();
    for a in TOKENS {
        for b in TOKENS {
            for c in TOKENS {
                lines.push([a, b, c].concat());
            }
        }
    }
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        usize::try_from(seed % n as u64).unwrap()
    };
    (0..240)
        .map(|_| {
            let count = 1 + next(6);
            let mut buf = Vec::new();
            for _ in 0..count {
                buf.extend_from_slice(&lines[next(lines.len())]);
                buf.extend_from_slice([&b"\n"[..], b"\r\n", b""][next(3)]);
            }
            buf
        })
        // Adjacent `<` and `>` tokens can spell an empty email; those are compared in
        // `empty_replaced_emails_are_skipped_where_c_fails`.
        .filter(|buf| !buf.windows(2).any(|w| w == b"<>"))
        .collect()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_mailmaps_match() {
    init();
    let mut c_mailmap = ptr::null_mut();
    // SAFETY: a writable slot.
    let c_rc = unsafe { ffi::git_mailmap_new(&raw mut c_mailmap) };
    let rust = Mailmap::new().map(rust_steps);
    assert_eq!(c_rc, 0);
    let c_steps = add_steps(c_mailmap);
    // SAFETY: the mailmap C built, freed once.
    unsafe { ffi::git_mailmap_free(c_mailmap) };
    assert_eq!(rust.unwrap(), c_steps);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn buffers_match() {
    init();
    let mut buffers = vec![
        MAILMAP.to_vec(),
        Vec::new(),
        b"A <> <a@x>\nB <> Name <b@x>\n".to_vec(),
        MAILMAP.repeat(3),
        b"Real <r@x> <a@x>\nOther <o@x> <a@x>\n".to_vec(),
    ];
    // BorrowSanitizer makes each parse slow: a sparser sample of prefixes and of the
    // generated buffers under it.
    let stride = if cfg!(bsan) { 11 } else { 1 };
    buffers.extend(
        (0..MAILMAP.len())
            .step_by(7 * stride)
            .map(|end| MAILMAP[..end].to_vec()),
    );
    let generated = generated_buffers();
    assert!(
        generated.len() > 150,
        "{} generated buffers",
        generated.len()
    );
    buffers.extend(generated.into_iter().step_by(stride));
    let (_, _, reference) = c_from_buffer(MAILMAP);
    assert!(mapped(&reference) > 10);
    for buf in &buffers {
        let (c_rc, c_err, c_steps) = c_from_buffer(buf);
        let rust = Mailmap::from_buffer(buf);
        let shown = String::from_utf8_lossy(buf);
        assert_eq!(c_rc, 0, "{shown:?}: {c_err:?}");
        assert_eq!(rust.map(rust_steps).ok(), Some(c_steps), "{shown:?}");
    }
}

/// Both sides refuse a NUL byte; C sets no error, Rust reports one (see the module
/// docs).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn nul_bytes_fail_on_both_sides() {
    init();
    for buf in [&b"\0"[..], b"A <a@x> <b@x>\n\0", b"\0\nA <a@x>"] {
        let (c_rc, c_err, c_steps) = c_from_buffer(buf);
        assert_eq!(c_rc, -1);
        assert_eq!(
            c_err,
            Some((ffi::GIT_ERROR_NONE.cast_signed(), "no error".to_owned()))
        );
        assert_eq!(
            c_steps,
            Vec::<Vec<(i32, Option<(String, String)>, Option<(i32, String)>)>>::new()
        );
        let error = Mailmap::from_buffer(buf).unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Invalid));
        assert_eq!(error.message(), c"mailmap buffer contains a NUL byte");
        assert_eq!(last(), error);
    }
}

/// A line replacing an empty email is skipped by the wrapper; the result equals C's
/// on the same buffer without those lines (see the module docs).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_replaced_emails_are_skipped_where_c_fails() {
    init();
    let cases: [(&[u8], &[u8]); 4] = [
        (b"Name <>\n", b""),
        (b"Name <>", b""),
        (
            b"A <a@x> B <>\nReal <r@x> <a@x>\n <> \nC <c@x> <>   # c\nD <> <b@x>\n",
            b"Real <r@x> <a@x>\nD <> <b@x>\n",
        ),
        (
            &[MAILMAP, b"Phil <>\r\nSome One <some@x> nick3 <>"].concat(),
            MAILMAP,
        ),
    ];
    for (with, without) in cases {
        let (c_rc, _, c_steps) = c_from_buffer(without);
        assert_eq!(c_rc, 0);
        let rust = Mailmap::from_buffer(with).map(rust_steps);
        assert_eq!(
            rust.ok(),
            Some(c_steps),
            "{:?}",
            String::from_utf8_lossy(with)
        );
    }
}

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes()).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A copy of libgit2's `mailmap` repository for `side` of variant `i`, its config
/// extended with `config`: the working directory and the git directory.
fn mailmap_repo(root: &Path, side: &str, i: usize, config: &str) -> (PathBuf, PathBuf) {
    let work = root.join(format!("{side}-{i}"));
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("c/tests/resources/mailmap"),
        &work,
    );
    let git = work.join(".git");
    fs::rename(work.join(".gitted"), &git).unwrap();
    let mut text = fs::read_to_string(git.join("config")).unwrap();
    text.push_str(config);
    fs::write(git.join("config"), text).unwrap();
    (work, git)
}

/// The C side of `from_repository` on the repository `open` opens: its steps.
fn c_from_repository(
    open: impl FnOnce(*mut *mut ffi::git_repository) -> c_int,
) -> Vec<Vec<Record>> {
    let mut repo = ptr::null_mut();
    assert_eq!(open(&raw mut repo), 0);
    let mut out = ptr::null_mut();
    // SAFETY: a writable slot and an open repository.
    let rc = unsafe { ffi::git_mailmap_from_repository(&raw mut out, repo) };
    assert_eq!(rc, 0);
    // SAFETY: opened above, freed once; the mailmap keeps nothing of it.
    unsafe { ffi::git_repository_free(repo) };
    let steps = add_steps(out);
    // SAFETY: the mailmap C built, freed once.
    unsafe { ffi::git_mailmap_free(out) };
    steps
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn repository_mailmaps_match() {
    init();
    let root = std::env::temp_dir().join(format!("crustify-equiv-mailmap-{}", std::process::id()));
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).unwrap();
    let override_file = root.join("file_override");
    fs::write(
        &override_file,
        b"File Override <phil@company.xx>\nOther Name <fileoverridename@company.xx>\n",
    )
    .unwrap();
    let nul_file = root.join("nul");
    fs::write(&nul_file, b"Nul <nul@x> <a@x>\n\0\n").unwrap();
    // Each setting, and whether a bare repository, which reads no working directory,
    // still finds a source.
    let configs = [
        (String::new(), true),
        (
            format!("[mailmap]\n\tfile = {}\n", override_file.display()),
            true,
        ),
        ("[mailmap]\n\tblob = HEAD:blob_override\n".to_owned(), true),
        (
            "[mailmap]\n\tblob = HEAD:missing\n\tfile = /nonexistent/mailmap\n".to_owned(),
            false,
        ),
        // A tree does not peel to a blob, and a source with a NUL byte is refused.
        (
            format!(
                "[mailmap]\n\tblob = HEAD^{{tree}}\n\tfile = {}\n",
                nul_file.display()
            ),
            false,
        ),
    ];
    for (i, (config, bare_loads)) in configs.iter().enumerate() {
        for bare in [false, true] {
            let (c_work, c_git) = mailmap_repo(&root, &format!("c-{bare}"), i, config);
            let (r_work, r_git) = mailmap_repo(&root, &format!("rust-{bare}"), i, config);
            let c_steps = c_from_repository(|out| {
                let path = c_path(if bare { &c_git } else { &c_work });
                if bare {
                    // SAFETY: a writable slot and a NUL-terminated path.
                    unsafe { ffi::git_repository_open_bare(out, path.as_ptr()) }
                } else {
                    // SAFETY: as above.
                    unsafe { ffi::git_repository_open(out, path.as_ptr()) }
                }
            });
            let repo = if bare {
                Repository::open_bare(&c_path(&r_git))
            } else {
                Repository::open(&c_path(&r_work))
            }
            .unwrap();
            let mailmap = Mailmap::from_repository(repo.as_ref()).unwrap();
            drop(repo);
            let loads = !bare || *bare_loads;
            assert_eq!(mapped(&c_steps) > 0, loads, "config {i}, bare {bare}");
            assert_eq!(rust_steps(mailmap), c_steps, "config {i}, bare {bare}");
        }
    }
    // A repository without paths: no working directory, no `HEAD`, no sources.
    let c_steps = c_from_repository(|out| {
        // SAFETY: a writable slot.
        unsafe { ffi::git_repository_new(out) }
    });
    let repo = Repository::new().unwrap();
    let mailmap = Mailmap::from_repository(repo.as_ref()).unwrap();
    assert_eq!(mapped(&c_steps), 0);
    assert_eq!(rust_steps(mailmap), c_steps);
    drop(fs::remove_dir_all(&root));
}

/// Equivalence of `MailmapMut::add_entry` and `MailmapRef::resolve` with
/// `git_mailmap_add_entry` and `git_mailmap_resolve`.
///
/// Each side builds its own mailmap from the same buffer, then runs one script of
/// entries: real names and emails present, absent and empty, entries replacing earlier
/// ones, name-specific entries beside nameless ones, emails differing only in case.
/// After every step both resolve the same probe identities, and the resolved strings,
/// or the step's code, error class and message, are compared. Each side also records
/// whether a resolved string is the probe's own or the mailmap's, by address.
///
/// Documented divergence, outside these comparisons: an empty replaced email is refused
/// by the wrapper with libgit2's own error, before libgit2 allocates an entry it then
/// leaks; the unit tests of `mailmap.rs` hold the refusal.
mod entries {
    use std::ffi::{CStr, c_char, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::mailmap::{Mailmap, MailmapRef};

    use super::{MAILMAP, c_error, init};

    type Entry = (
        Option<&'static CStr>,
        Option<&'static CStr>,
        Option<&'static CStr>,
        &'static CStr,
    );

    fn script() -> Vec<Entry> {
        vec![
            (
                Some(c"Real"),
                Some(c"real@example.com"),
                None,
                c"a@example.com",
            ),
            (Some(c"Named"), None, Some(c"A Name"), c"a@example.com"),
            (
                None,
                Some(c"only-email@example.com"),
                None,
                c"b@example.com",
            ),
            (Some(c""), Some(c""), Some(c""), c"c@example.com"),
            (Some(c"Replaced"), None, None, c"a@example.com"),
            (Some(c"Case"), None, None, c"A@example.com"),
            (None, None, None, c"d@example.com"),
            (
                Some(c"Other Real"),
                Some(c"other@example.com"),
                Some(c"Other Old"),
                c"other.old@example.com",
            ),
            (Some(c"Z"), None, Some(c"z"), c"z@x"),
        ]
    }

    fn probes() -> Vec<(&'static CStr, &'static CStr)> {
        vec![
            (c"Anyone", c"a@example.com"),
            (c"A Name", c"a@example.com"),
            (c"a name", c"a@example.com"),
            (c"X", c"A@example.com"),
            (c"X", c"b@example.com"),
            (c"X", c"c@example.com"),
            (c"X", c"d@example.com"),
            (c"Other Old", c"other.old@example.com"),
            (c"Someone Else", c"other.old@example.com"),
            (c"X", c"old@example.com"),
            (c"", c""),
            (c"z", c"z@x"),
        ]
    }

    /// A probe's resolution: the strings, and whether each is the probe's own.
    type Resolved = (String, String, bool, bool);

    fn record(
        name: &CStr,
        email: &CStr,
        real_name: *const c_char,
        real_email: *const c_char,
    ) -> Resolved {
        // SAFETY: both point at the probe's strings or at the mailmap's, live here.
        let cstr = unsafe { CStr::from_ptr(real_name) };
        // SAFETY: as above.
        let (n, e) = unsafe { (cstr, CStr::from_ptr(real_email)) };
        (
            n.to_string_lossy().into_owned(),
            e.to_string_lossy().into_owned(),
            real_name == name.as_ptr(),
            real_email == email.as_ptr(),
        )
    }

    fn c_resolve(mailmap: *const ffi::git_mailmap) -> Vec<Resolved> {
        probes()
            .into_iter()
            .map(|(name, email)| {
                let (mut n, mut e) = (ptr::null(), ptr::null());
                // SAFETY: two writable slots, a live mailmap, NUL-terminated probes.
                let rc = unsafe {
                    ffi::git_mailmap_resolve(
                        &raw mut n,
                        &raw mut e,
                        mailmap,
                        name.as_ptr(),
                        email.as_ptr(),
                    )
                };
                assert_eq!(rc, 0);
                record(name, email, n, e)
            })
            .collect()
    }

    fn rust_resolve(mailmap: MailmapRef<'_>) -> Vec<Resolved> {
        probes()
            .into_iter()
            .map(|(name, email)| {
                let (n, e) = mailmap.resolve(name, email);
                record(name, email, n.as_ptr(), e.as_ptr())
            })
            .collect()
    }

    type Step = (Result<(), (c_int, String)>, Vec<Resolved>);

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entries_and_lookups_match_c() {
        init();
        let mut c_map = ptr::null_mut();
        // SAFETY: a writable slot and a live buffer.
        let rc = unsafe {
            ffi::git_mailmap_from_buffer(&raw mut c_map, MAILMAP.as_ptr().cast(), MAILMAP.len())
        };
        assert_eq!(rc, 0);
        let mut rust_map = Mailmap::from_buffer(MAILMAP).unwrap();
        let mut c_steps: Vec<Step> = vec![(Ok(()), c_resolve(c_map))];
        let mut rust_steps: Vec<Step> = vec![(Ok(()), rust_resolve(rust_map.as_ref()))];
        let opt = |s: Option<&CStr>| s.map_or(ptr::null(), CStr::as_ptr);
        for (real_name, real_email, name, email) in script() {
            // SAFETY: the live mailmap, NULL or NUL-terminated strings, a non-empty email.
            let rc = unsafe {
                ffi::git_mailmap_add_entry(
                    c_map,
                    opt(real_name),
                    opt(real_email),
                    opt(name),
                    email.as_ptr(),
                )
            };
            let c_rc = if rc < 0 {
                Err((rc, c_error().1))
            } else {
                Ok(())
            };
            c_steps.push((c_rc, c_resolve(c_map)));
            let r = rust_map
                .as_mut()
                .add_entry(real_name, real_email, name, email);
            let r = r.map_err(|e| (-1, e.message().to_string_lossy().into_owned()));
            rust_steps.push((r, rust_resolve(rust_map.as_ref())));
            assert_eq!(rust_steps.last(), c_steps.last(), "{email:?}");
        }
        // SAFETY: the C side's mailmap, freed once.
        unsafe { ffi::git_mailmap_free(c_map) };
        assert_eq!(rust_steps, c_steps);
    }
}

/// Equivalence of [`MailmapRef::resolve_signature`] with
/// `git_mailmap_resolve_signature`.
///
/// Each side builds its own mailmap from libgit2's reference file and holds its own
/// copies of a list of signatures: parsed from signature lines (zones east, west and
/// `-0000`, whose sign `git_signature_new` alone would lose) and built over borrowed
/// strings (C: a `git_signature` on the stack; Rust: a [`SignatureView`]), including
/// strings libgit2 trims, strings holding angle brackets and strings that trim to
/// nothing. Every signature is resolved, then entries are added to both mailmaps
/// (among them a real name with angle brackets and one that trims to nothing) and every
/// signature is resolved again. Each resolution records the resolved name, email, time,
/// offset and sign, or the failure's code (always -1) and the error's class and message;
/// the records are compared after each step.
///
/// Outside these comparisons: C also takes a NULL mailmap (resolving to the identity)
/// and a NULL signature (success without a result), which the wrapper's arguments
/// cannot carry, and a signature missing its name or email, which the wrapper refuses
/// before libgit2's assertion (`mailmap.rs`'s unit tests).
///
/// [`MailmapRef::resolve_signature`]: libgit2::libgit2::object::mailmap::MailmapRef::resolve_signature
/// [`SignatureView`]: libgit2::libgit2::core::types::SignatureView
mod signatures {
    use std::ffi::{CStr, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{Signature, SignatureRef, SignatureView, Time};
    use libgit2::libgit2::object::mailmap::{Mailmap, MailmapRef};

    use super::{MAILMAP, c_error, init};

    /// A resolved signature `(name, email, seconds, offset, sign)`, or the failure.
    type Resolved = Result<(Vec<u8>, Vec<u8>, i64, c_int, u8), (c_int, c_int, String)>;

    const LINES: [&CStr; 6] = [
        c"nick1 <bugs@company.xx> 1700000000 +0200",
        c"Who <me@company.xx> 7 -0000",
        c"Joseph <bugs@company.xx> 0 -0130",
        c"  Spaced  <cto@coompany.xx> 1 +0000",
        c"Brackets <a<b@x> 2 +0100",
        c"<nick2@company.xx> 3 +0000",
    ];

    const VIEWS: [(&CStr, &CStr, i64, c_int); 7] = [
        (c"nick2", c"bugs@company.xx", 5, 60),
        (c"Some Name", c"untracked@company.xx", -5, -60),
        (c"  , Trimmed ; ", c"\"me@company.xx\"", 0, 0),
        (c"<Bad>", c"bad@x", 0, 0),
        (c"Fine", c"x>y@z", 0, 0),
        (c" ", c"empty@x", 0, 0),
        (c"New", c"new@x", 9, 9),
    ];

    const ENTRIES: [(&CStr, &CStr, &CStr); 4] = [
        (c"Real <New>", c"real@x", c"new@x"),
        (c" ;", c"blank@x", c"bad@x"),
        (c"Newer", c"newer@x", c"new@x"),
        (c"Fixed", c"fixed@x", c"empty@x"),
    ];

    fn c_signatures() -> Vec<*mut ffi::git_signature> {
        LINES
            .iter()
            .map(|line| {
                let mut sig = ptr::null_mut();
                // SAFETY: a writable slot and a NUL-terminated line.
                let rc = unsafe { ffi::git_signature_from_buffer(&raw mut sig, line.as_ptr()) };
                assert_eq!(rc, 0, "{line:?}");
                sig
            })
            .collect()
    }

    fn c_views() -> Vec<ffi::git_signature> {
        VIEWS
            .iter()
            .map(|&(name, email, time, offset)| ffi::git_signature {
                name: name.as_ptr().cast_mut(),
                email: email.as_ptr().cast_mut(),
                when: ffi::git_time {
                    time,
                    offset,
                    sign: (if offset < 0 { b'-' } else { b'+' }).cast_signed(),
                },
            })
            .collect()
    }

    fn c_resolve(map: *const ffi::git_mailmap, sig: *const ffi::git_signature) -> Resolved {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot, a live mailmap and a live signature with both strings.
        let rc = unsafe { ffi::git_mailmap_resolve_signature(&raw mut out, map, sig) };
        if rc < 0 {
            let (class, message) = c_error();
            return Err((rc, class, message));
        }
        {
            // SAFETY: on success a fresh signature, read and freed once.
            let r = unsafe { &*out };
            // SAFETY: as above.
            let cstr = unsafe { CStr::from_ptr(r.name) };
            // SAFETY: as above.
            let resolved = unsafe {
                (
                    cstr.to_bytes().to_vec(),
                    CStr::from_ptr(r.email).to_bytes().to_vec(),
                    r.when.time,
                    r.when.offset,
                    r.when.sign.cast_unsigned(),
                )
            };
            // SAFETY: as above.
            unsafe { ffi::git_signature_free(out) };
            Ok(resolved)
        }
    }

    fn rust_resolve(map: MailmapRef<'_>, sig: SignatureRef<'_>) -> Resolved {
        match map.resolve_signature(sig) {
            Ok(out) => {
                let r = out.as_ref();
                Ok((
                    r.name().unwrap().to_bytes().to_vec(),
                    r.email().unwrap().to_bytes().to_vec(),
                    r.when().seconds(),
                    r.when().offset_minutes(),
                    r.when().sign(),
                ))
            }
            Err(e) => Err((
                -1,
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            )),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn resolutions_match_c_before_and_after_new_entries() {
        init();
        let mut c_map = ptr::null_mut();
        // SAFETY: a writable slot and a live buffer.
        let rc = unsafe {
            ffi::git_mailmap_from_buffer(&raw mut c_map, MAILMAP.as_ptr().cast(), MAILMAP.len())
        };
        assert_eq!(rc, 0);
        let mut rust_map = Mailmap::from_buffer(MAILMAP).unwrap();
        let c_parsed = c_signatures();
        let c_viewed = c_views();
        let rust_parsed: Vec<_> = LINES.iter().map(|l| Signature::parse(l).unwrap()).collect();
        let rust_viewed: Vec<_> = VIEWS
            .iter()
            .map(|&(name, email, time, offset)| {
                SignatureView::new(name, email, Time::new(time, offset))
            })
            .collect();
        let c_all = || {
            let mut out: Vec<Resolved> = c_parsed.iter().map(|&s| c_resolve(c_map, s)).collect();
            out.extend(c_viewed.iter().map(|s| c_resolve(c_map, s)));
            out
        };
        let rust_all = |map: MailmapRef<'_>| {
            let mut out: Vec<Resolved> = rust_parsed
                .iter()
                .map(|s| rust_resolve(map, s.as_ref()))
                .collect();
            out.extend(rust_viewed.iter().map(|s| rust_resolve(map, s.as_ref())));
            out
        };
        let first = c_all();
        assert_eq!(rust_all(rust_map.as_ref()), first, "before any entry");
        assert!(first.iter().any(Result::is_err) && first.iter().any(Result::is_ok));
        assert!(first.iter().any(|r| matches!(r, Ok((.., b'-')))));
        for (i, (real_name, real_email, email)) in ENTRIES.into_iter().enumerate() {
            // SAFETY: the live mailmap and NUL-terminated strings, `email` non-empty.
            let rc = unsafe {
                ffi::git_mailmap_add_entry(
                    c_map,
                    real_name.as_ptr(),
                    real_email.as_ptr(),
                    ptr::null(),
                    email.as_ptr(),
                )
            };
            assert_eq!(rc, 0);
            rust_map
                .as_mut()
                .add_entry(Some(real_name), Some(real_email), None, email)
                .unwrap();
            assert_eq!(rust_all(rust_map.as_ref()), c_all(), "after entry {i}");
        }
        for sig in c_parsed {
            // SAFETY: each parsed signature, freed once.
            unsafe { ffi::git_signature_free(sig) };
        }
        // SAFETY: the C side's mailmap, freed once.
        unsafe { ffi::git_mailmap_free(c_map) };
    }
}
