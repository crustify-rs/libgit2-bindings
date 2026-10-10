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
//! - a buffer with a NUL byte fails on both sides, but libgit2 sets no error, while the
//!   wrapper reports `GIT_ERROR_INVALID`, "mailmap buffer contains a NUL byte";
//! - a line whose replaced email is empty (`Name <>`) makes libgit2 fail the whole
//!   buffer with a failed argument assertion and leak the entry it began; the wrapper
//!   skips the line, as libgit2 skips every other line that is not an entry. C is not
//!   run on such lines here (the leak fails LeakSanitizer); the Rust result is compared
//!   with C's on the same buffer without them, and the classification itself with C's
//!   refusals in the crate's unit tests.

use std::ffi::{CStr, CString, c_char, c_int};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

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
    let error = unsafe { &*ffi::git_error_last() };
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
                    &mut real_name,
                    &mut real_email,
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
            let resolved = unsafe {
                (
                    CStr::from_ptr(real_name).to_string_lossy().into_owned(),
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
    let rc = unsafe { ffi::git_mailmap_from_buffer(&mut out, data, buf.len()) };
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
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
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
    let c_rc = unsafe { ffi::git_mailmap_new(&mut c_mailmap) };
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
    buffers.extend(
        (0..MAILMAP.len())
            .step_by(7)
            .map(|end| MAILMAP[..end].to_vec()),
    );
    let generated = generated_buffers();
    assert!(
        generated.len() > 150,
        "{} generated buffers",
        generated.len()
    );
    buffers.extend(generated);
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
            Some((ffi::GIT_ERROR_NONE as c_int, "no error".to_owned()))
        );
        assert!(c_steps.is_empty());
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
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/resources/mailmap"),
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
    assert_eq!(open(&mut repo), 0);
    let mut out = ptr::null_mut();
    // SAFETY: a writable slot and an open repository.
    let rc = unsafe { ffi::git_mailmap_from_repository(&mut out, repo) };
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
    let _ = fs::remove_dir_all(&root);
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
                // SAFETY: a writable slot and a NUL-terminated path.
                unsafe {
                    if bare {
                        ffi::git_repository_open_bare(out, path.as_ptr())
                    } else {
                        ffi::git_repository_open(out, path.as_ptr())
                    }
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
    let _ = fs::remove_dir_all(&root);
}
