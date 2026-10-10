//! UB tests for `git2/mailmap.h`'s [`Mailmap`] owner and handles.
//!
//! A [`MailmapBox`] owns a C mailmap, its entry vector and every entry's strings, and
//! frees them once through `git_mailmap_free` (ASan flags a double free or a use after
//! free, LSan a leak). Each constructor is driven here: [`Mailmap::new`],
//! [`Mailmap::from_buffer`] over well-formed, malformed and adversarial buffers
//! (including the lines that make libgit2 leak, which the wrapper leaves out), and
//! [`Mailmap::from_repository`] over a working directory, a bare repository, the
//! `mailmap.file` and `mailmap.blob` settings, and a repository without paths. A
//! mailmap keeps nothing of what built it, so it is used and dropped after its buffer
//! and its repository. It is `Send` and `Sync`: boxes are built, shared and dropped
//! across threads (TSan). The lookups that read it land with their own run; the
//! handles here are only reborrowed. Illegal paths (a handle outliving its box, a
//! shared handle next to an exclusive one) are in `compile_fail/`.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::thread;

use libgit2::libgit2::object::mailmap::{Mailmap, MailmapBox, MailmapRef};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
use libgit2::libgit2::util::errors::ErrorClass;

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

/// A scratch directory of its own under the temp dir, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("crustify-mailmap-ub-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes()).unwrap()
}

/// A copy of libgit2's `mailmap` repository (a working directory with a `.mailmap`, and
/// `.mailmap` and `blob_override` committed at `HEAD`), its config extended with
/// `config`. Its git directory is returned too.
fn mailmap_repo(scratch: &Scratch, name: &str, config: &str) -> (PathBuf, PathBuf) {
    let work = scratch.0.join(name);
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

/// Reborrows `mailmap`'s handles the ways the safe API allows.
fn reborrow(mailmap: &mut MailmapBox) {
    let shared: MailmapRef<'_> = mailmap.as_ref();
    let copy = shared;
    let printed = format!("{shared:?}");
    assert_eq!(printed, format!("{copy:?}"));
    let mut exclusive = mailmap.as_mut();
    let inner = exclusive.as_mut();
    assert_eq!(format!("{inner:?}"), printed);
    assert_eq!(
        format!("{:?}", exclusive.as_ref()),
        format!("{exclusive:?}")
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_constructor_builds_an_owner_freed_once() {
    let mut boxes = vec![
        Mailmap::new().unwrap(),
        MailmapBox::new().unwrap(),
        Mailmap::from_buffer(MAILMAP).unwrap(),
        Mailmap::from_buffer(b"").unwrap(),
        Mailmap::from_buffer(b"\n\n# only comments\n").unwrap(),
        Mailmap::from_repository(Repository::new().unwrap().as_ref()).unwrap(),
    ];
    for mailmap in &mut boxes {
        reborrow(mailmap);
    }
    // Dropped in reverse, then in order, to vary the allocator's free lists.
    while let Some(last) = boxes.pop() {
        drop(last);
        if let Some(first) = (!boxes.is_empty()).then(|| boxes.remove(0)) {
            drop(first);
        }
    }
}

/// Buffers that stress the parser: unterminated lines, stray brackets, comments
/// inside emails, huge names, CRLF endings, bytes above 0x7f, duplicates that
/// replace earlier entries (freeing them), and empty emails in every position. None
/// may read out of bounds, leak or fail, but those with a NUL byte.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adversarial_buffers_parse_without_ub() {
    // BorrowSanitizer makes libgit2's byte-by-byte parse of a long name, and each
    // entry's allocations, slow: smaller inputs under it.
    let long_name = "N".repeat(if cfg!(bsan) { 1 << 10 } else { 1 << 16 });
    let buffers: Vec<Vec<u8>> = vec![
        b"<".to_vec(),
        b">".to_vec(),
        b"<>".to_vec(),
        b"Name <>".to_vec(),
        b"Name <> ".to_vec(),
        b"<> <>\n<a@x> <>\r\n".to_vec(),
        b"A <a@x> B <>\nC <c@x> <d@x>\n".to_vec(),
        b"Real <r@x> <o@x>\nOther <o2@x> <o@x>\nReal <r@x> <o@x>\n".to_vec(),
        b"A <a#x> <b@x>\nB <b@x> # <c@x>\n".to_vec(),
        b"A <a@x> B <b@x> C <c@x>\n".to_vec(),
        b"\xff\xfe <\xc3\xa9@x> \x80 <\x81@x>\r\n".to_vec(),
        format!("{long_name} <long@x> {long_name} <old@x>").into_bytes(),
        MAILMAP.repeat(if cfg!(bsan) { 4 } else { 64 }),
        MAILMAP[..MAILMAP.len() - 3].to_vec(),
    ];
    let mut kept = Vec::new();
    for buf in &buffers {
        kept.push(Mailmap::from_buffer(buf).unwrap());
    }
    // Every prefix of the reference file, each cut mid-line or mid-email (a sample of
    // them under BorrowSanitizer).
    let stride = if cfg!(bsan) { 37 } else { 1 };
    for end in (0..MAILMAP.len()).step_by(stride) {
        drop(Mailmap::from_buffer(&MAILMAP[..end]).unwrap());
    }
    drop(buffers);
    for mut mailmap in kept {
        reborrow(&mut mailmap);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn nul_bytes_are_refused_without_a_leak() {
    for buf in [&b"\0"[..], b"A <a@x>\0", b"Name <>\n\0", b"\0Name <>"] {
        let error = Mailmap::from_buffer(buf).unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Invalid));
    }
}

/// A repository's mailmap, from each source libgit2 reads, outlives the repository
/// (ASan flags a read of it, LSan a leak of what loading allocated).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn repository_mailmaps_outlive_their_repository() {
    let scratch = Scratch::new("repos");
    let override_file = scratch.0.join("override");
    fs::write(&override_file, b"File Override <phil@company.xx>\n").unwrap();
    let nul_file = scratch.0.join("nul");
    fs::write(&nul_file, b"Nul <nul@x> <a@x>\n\0\n").unwrap();
    let configs = [
        String::new(),
        format!("[mailmap]\n\tfile = {}\n", override_file.display()),
        "[mailmap]\n\tblob = HEAD:blob_override\n".to_owned(),
        "[mailmap]\n\tblob = HEAD:missing\n\tfile = /nonexistent/mailmap\n".to_owned(),
        // A tree does not peel to a blob, and a source with a NUL byte is refused.
        format!(
            "[mailmap]\n\tblob = HEAD^{{tree}}\n\tfile = {}\n",
            nul_file.display()
        ),
    ];
    let mut mailmaps = Vec::new();
    for (i, config) in configs.iter().enumerate() {
        let (work, git) = mailmap_repo(&scratch, &format!("repo-{i}"), config);
        let repo: RepositoryBox = Repository::open(&c_path(&work)).unwrap();
        mailmaps.push(Mailmap::from_repository(repo.as_ref()).unwrap());
        // Twice from one repository: its config cache is filled once, then reused.
        mailmaps.push(Mailmap::from_repository(repo.as_ref()).unwrap());
        drop(repo);
        let bare = Repository::open_bare(&c_path(&git)).unwrap();
        mailmaps.push(Mailmap::from_repository(bare.as_ref()).unwrap());
        drop(bare);
    }
    drop(scratch);
    for mut mailmap in mailmaps {
        reborrow(&mut mailmap);
    }
}

/// Boxes are built on some threads, shared by reference across others, and dropped on
/// yet others (TSan flags a race, ASan a free into the wrong state).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn mailmaps_cross_threads() {
    let built: Vec<MailmapBox> = (0..4)
        .map(|i| {
            thread::spawn(move || {
                let mut buf = MAILMAP.to_vec();
                buf.extend(format!("T{i} <t{i}@x> <old{i}@x>\n").bytes());
                Mailmap::from_buffer(&buf).unwrap()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for mailmap in &built {
                    let shared = mailmap.as_ref();
                    assert!(!format!("{shared:?}").is_empty());
                }
            });
        }
    });
    let handles: Vec<_> = built
        .into_iter()
        .map(|mailmap| thread::spawn(move || drop(mailmap)))
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    // An exclusive handle moves to a scoped thread and back.
    let mut sole = Mailmap::new().unwrap();
    thread::scope(|s| {
        let mut exclusive = sole.as_mut();
        s.spawn(move || format!("{:?}", exclusive.as_mut()));
    });
    reborrow(&mut sole);
}

/// UB tests for the mailmap's lookups and entries: `MailmapRef::resolve` and
/// `MailmapMut::add_entry`.
///
/// `resolve` hands back either the strings it was given or an entry's, which the
/// mailmap owns: each is read after the other side is overwritten (ASan flags a string
/// read past its end or after it was freed). `add_entry` copies its strings, which are
/// exact-length heap strings freed right after the call (BSan, ASan), and frees the entry
/// a later one for the same identity replaces, which no lookup still holds (the borrow
/// checker; `compile_fail` doctest). An empty replaced email, which libgit2 leaks on,
/// is refused (LSan). Lookups run on many threads through shared handles at once, and
/// a mailmap filled on one thread is read on others (TSan).
mod lookups {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::libgit2::object::mailmap::Mailmap;
    use libgit2::libgit2::util::errors::ErrorClass;

    fn exact(s: &str) -> Box<CStr> {
        CString::new(s).unwrap().into_boxed_c_str()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entries_are_copied_and_resolved_strings_borrow_both_sides() {
        let mut mailmap = Mailmap::new().unwrap();
        for i in 0..40 {
            let (real, email, old) = (
                exact(&format!("Real {i}")),
                exact(&format!("real{i}@example.com")),
                exact(&format!("old{}@example.com", i % 10)),
            );
            let name = (i % 3 == 0).then(|| exact(&format!("Old {i}")));
            mailmap
                .as_mut()
                .add_entry(Some(&real), Some(&email), name.as_deref(), &old)
                .unwrap();
            drop((real, email, old, name));
        }
        let r = mailmap.as_ref();
        let (name, email) = (exact("Old 30"), exact("old0@example.com"));
        let (real_name, real_email) = r.resolve(&name, &email);
        assert_eq!((real_name, real_email), (c"Real 30", c"real30@example.com"));
        let (other, other_email) = (exact("Someone"), exact("nobody@example.com"));
        let (same, same_email) = r.resolve(&other, &other_email);
        assert_eq!(same.as_ptr(), other.as_ptr());
        assert_eq!(same_email.as_ptr(), other_email.as_ptr());
        // The email-only entry for `old1@` is the last one added for it.
        assert_eq!(
            r.resolve(c"X", c"old1@example.com"),
            (c"Real 31", c"real31@example.com")
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_and_replaced_entries_leave_nothing_behind() {
        let mut mailmap = Mailmap::new().unwrap();
        let mut m = mailmap.as_mut();
        let e = m.add_entry(Some(c"R"), None, Some(c"N"), c"").unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        for round in 0..8 {
            let real = exact(&"R".repeat(round + 1));
            m.add_entry(Some(&real), None, Some(c"N"), c"n@example.com")
                .unwrap();
            m.add_entry(Some(&real), None, None, c"n@example.com")
                .unwrap();
        }
        let r = mailmap.as_ref();
        assert_eq!(r.resolve(c"N", c"n@example.com").0, c"RRRRRRRR");
        assert_eq!(r.resolve(c"M", c"n@example.com").0, c"RRRRRRRR");
        assert_eq!(r.resolve(c"N", c""), (c"N", c""));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_on_many_threads() {
        let mut mailmap = Mailmap::new().unwrap();
        for i in 0..16 {
            let email = exact(&format!("u{i}@example.com"));
            mailmap
                .as_mut()
                .add_entry(Some(c"Real"), None, None, &email)
                .unwrap();
        }
        let mailmap = thread::spawn(move || mailmap).join().unwrap();
        thread::scope(|s| {
            for t in 0..4 {
                let r = mailmap.as_ref();
                s.spawn(move || {
                    for i in 0..64 {
                        let email = exact(&format!("u{}@example.com", (i + t) % 20));
                        let (name, _) = r.resolve(c"Who", &email);
                        let expected: &CStr = if (i + t) % 20 < 16 { c"Real" } else { c"Who" };
                        assert_eq!(name, expected);
                    }
                });
            }
        });
    }
}
