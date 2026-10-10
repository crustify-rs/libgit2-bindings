//! Equivalence of `git2/tag.h`'s `name_is_valid` with `git_tag_name_is_valid`, of the
//! tag object's owner and constructors with their C lifecycle in [`tag_object`], and of
//! the routines over a repository's tags in [`repository_tags`].
//!
//! Each probe hands the same name, in two independently owned `CString`s, to C and to
//! Rust, and compares the outcome (C's return code and `valid` against the `Result`)
//! and the calling thread's last error after the call: both leave
//! `GIT_ERROR_REFERENCE`, naming the reference, when `refs/tags/<name>` is what fails,
//! and nothing when the tag rules (a leading `-`, `HEAD`) refuse the name first. The
//! probes cover named edge cases and every name of up to three characters over the
//! characters the rules tell apart. A multi-call sequence alternates valid and invalid
//! names without clearing the error, comparing after each step: a valid name leaves
//! the previous error in place on both sides.
//!
//! Outside these comparisons: C also takes a NULL name, reporting it invalid; the
//! wrapper takes a `&CStr`, which is never NULL. Out-of-memory failures are compared in
//! the wrapper's unit tests, which install a refusing allocator.

use std::ffi::{CStr, CString, c_int};

use libgit2::ffi;
use libgit2::libgit2::object::tag::name_is_valid;

/// Initialises libgit2; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The calling thread's last error class and message.
fn c_last() -> (c_int, String) {
    // SAFETY: `git_error_last` never returns NULL after init, and its message is a
    // NUL-terminated string, both live until the next libgit2 call on this thread,
    // after the copy.
    unsafe {
        let e = ffi::git_error_last();
        (
            (*e).klass,
            CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
        )
    }
}

fn clear() {
    // SAFETY: no arguments; libgit2 is initialised.
    unsafe { ffi::git_error_clear() }
}

/// C's outcome on its own copy of `name`, as `Ok(valid)` or `Err(rc)`, and the error it
/// leaves.
fn c_check(name: &CStr) -> (Result<bool, c_int>, (c_int, String)) {
    let copy = name.to_owned();
    let mut valid: c_int = -1;
    // SAFETY: `valid` is a writable `int`; `copy` is a NUL-terminated string, read for
    // the call only.
    let rc = unsafe { ffi::git_tag_name_is_valid(&mut valid, copy.as_ptr()) };
    let outcome = if rc < 0 {
        Err(rc)
    } else {
        assert!(valid == 0 || valid == 1, "C wrote valid = {valid}");
        Ok(valid != 0)
    };
    (outcome, c_last())
}

/// Rust's outcome on its own copy of `name`, and the error it leaves.
fn rust_check(name: &CStr) -> (Result<bool, c_int>, (c_int, String)) {
    let copy = name.to_owned();
    let outcome = name_is_valid(&copy).map_err(|e| e.raw_class());
    (outcome, c_last())
}

fn assert_equivalent(name: &CStr) {
    clear();
    let c = c_check(name);
    assert!(c.0.is_ok(), "C failed on {name:?}");
    clear();
    assert_eq!(rust_check(name), c, "{name:?}");
}

/// Names around every rule: the tag rules, each refname rule, and long ones.
fn named() -> Vec<CString> {
    let names: [&[u8]; 39] = [
        b"",
        b"v1.0",
        b"-v1.0",
        b"v-1",
        b"HEAD",
        b"HEADS",
        b"head",
        b"HEAD/x",
        b"x/HEAD",
        b"@",
        b"v@",
        b"a@{b",
        b"a{b",
        b"a..b",
        b".a",
        b"a.",
        b"a/.b",
        b"a/b.",
        b"a.lock",
        b"a.lock/b",
        b"a/b.lock",
        b"a//b",
        b"/a",
        b"a/",
        b"a b",
        b"a\tb",
        b"a~b",
        b"a^b",
        b"^a",
        b"a:b",
        b"a?b",
        b"a*b",
        b"a[b",
        b"a\\b",
        b"a\x7fb",
        b"\xc3\xa9t\xc3\xa9",
        b"\xff\x80",
        b"refs/heads/main",
        b"release/2024/q1",
    ];
    let mut names: Vec<CString> = names.iter().map(|n| CString::new(*n).unwrap()).collect();
    names.push(CString::new(vec![b'a'; 5000]).unwrap());
    names.push(CString::new("a/".repeat(500) + "b").unwrap());
    names
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn named_edge_cases() {
    init();
    for name in named() {
        assert_equivalent(&name);
    }
}

/// Every name of up to three characters over the characters the rules tell apart.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn short_names_exhaustively() {
    init();
    const CHARS: &[u8] = b"aH.-/@{*^ ~k";
    let mut names = vec![Vec::new()];
    for len in 1..=3 {
        let longer: Vec<Vec<u8>> = names
            .iter()
            .filter(|n: &&Vec<u8>| n.len() == len - 1)
            .flat_map(|n| CHARS.iter().map(move |&c| [&n[..], &[c]].concat()))
            .collect();
        names.extend(longer);
    }
    for name in names {
        assert_equivalent(&CString::new(name).unwrap());
    }
}

/// Without clearing the error between calls, C and Rust leave the same error after
/// each step.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn alternating_sequence_without_clearing() {
    init();
    let sequence: [&CStr; 10] = [
        c"v1", c"a..b", c"v2", c"-x", c"HEAD", c"b..c", c"v3", c"a.lock", c"", c"v4",
    ];
    clear();
    let c: Vec<_> = sequence.iter().map(|n| c_check(n)).collect();
    clear();
    let rust: Vec<_> = sequence.iter().map(|n| rust_check(n)).collect();
    for ((name, c), rust) in sequence.iter().zip(&c).zip(&rust) {
        assert_eq!(rust, c, "{name:?}");
    }
    // The reference error from "a..b" survives the valid "v2".
    assert_eq!(c[2].1.0, ffi::GIT_ERROR_REFERENCE as c_int);
    assert!(c[2].1.1.contains("refs/tags/a..b"));
}

mod tag_object {
    //! Equivalence of `git2/tag.h`'s tag owner and constructors with the C routines they
    //! wrap.
    //!
    //! [`Tag::lookup`] must find what `git_tag_lookup` finds, and [`Tag::lookup_prefix`]
    //! what `git_tag_lookup_prefix` does: the same tag (its id and type, and what it
    //! peels to, as C reads them) or the same failure (code, error class and message).
    //! Each side opens the repository on its own and runs one script of lookups, in
    //! order: every tag of `testrepo.git`, ids of commits, trees and blobs, missing and
    //! zero ids, prefixes of 0 to past 40 digits, ambiguous prefixes, and repeats. After
    //! every step the sides are compared, and so is which earlier result each new tag is
    //! the same object as (the cache hands back the tag it holds). Then each side copies
    //! every result (`git_tag_dup`; `clone`, [`TagRef::to_tag`], and a hand-over to an
    //! object owner and back on the Rust side), releases the originals, and compares the
    //! copies, and the lookups after the releases. A repository without a git directory
    //! is compared the same way, and so is one holding tags libgit2 cannot parse beside
    //! well-formed ones.
    //!
    //! Documented divergence, outside these comparisons: an id of another type than the
    //! repository's is refused by the wrappers (code `-1`, `GIT_ERROR_INVALID`), where the
    //! C routines format it out of bounds; the unit tests of `tag.rs` hold the refusal.
    //!
    //! [`TagRef::to_tag`]: libgit2::libgit2::object::tag::TagRef::to_tag

    use std::ffi::{CStr, CString, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::tag::{Tag, TagArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    use super::{c_last, init};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git"
    );

    /// Every tag of `testrepo.git`: `test` (of tag `7b43849`), `e90810b`, `hard_tag`,
    /// `taggerless` and `annotated_tag_to_blob`.
    const TAGS: [&str; 5] = [
        "b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
        "7b4384978d2493e851f9cca7858815fac9b10980",
        "849a5e34a26815e821f865b8479f5815a47af0fe",
        "4a23e2e65ad4e31c4c9db7dc746650bfad082679",
        "521d87c1ec3aef9824daf6d96cc0ae3710766d91",
    ];
    const COMMIT: &str = "e90810b8df3e80c413d903f631643c716887138d";
    const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
    const MISSING: &str = "dededededededededededededededededededede";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    /// What a lookup gave: the tag's id and type and what it peels to (or the code the
    /// peel failed with), as C reads them, or the failure.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Found(
            String,
            ffi::git_object_t,
            Result<(String, ffi::git_object_t), c_int>,
        ),
        Failed(c_int, c_int, String),
    }

    fn hex_of(id: *const ffi::git_oid) -> String {
        let mut out = [0u8; 65];
        // SAFETY: a live id, formatted into 65 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// C's view of a live tag.
    fn found(tag: *mut ffi::git_tag) -> Outcome {
        let object = tag.cast::<ffi::git_object>();
        let mut target = ptr::null_mut();
        // SAFETY: `tag` is a live tag, a `git_object` of type tag, whose repository is
        // live; its id is formatted at once, and the peeled target released once.
        unsafe {
            let peeled = match ffi::git_object_peel(&mut target, object, ffi::GIT_OBJECT_ANY) {
                rc if rc < 0 => Err(rc),
                _ => {
                    let view = (
                        hex_of(ffi::git_object_id(target)),
                        ffi::git_object_type(target),
                    );
                    ffi::git_object_free(target);
                    Ok(view)
                }
            };
            Outcome::Found(
                hex_of(ffi::git_object_id(object)),
                ffi::git_object_type(object),
                peeled,
            )
        }
    }

    /// One lookup of the script: an id (or prefix) in hex, and a length (`None` for the
    /// full lookup).
    #[derive(Clone, Copy, Debug)]
    struct Step<'a> {
        hex: &'a str,
        len: Option<usize>,
    }

    /// The C side: the raw repository and the tags it looked up.
    struct CSide {
        repo: *mut ffi::git_repository,
        held: Vec<*mut ffi::git_tag>,
    }

    impl CSide {
        fn open(path: &str) -> Self {
            let path = CString::new(path).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: a writable slot and a NUL-terminated path; no ceiling directories.
            let rc = unsafe {
                ffi::git_repository_open_ext(
                    &mut repo,
                    path.as_ptr(),
                    ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                    ptr::null(),
                )
            };
            assert_eq!(rc, 0);
            Self {
                repo,
                held: Vec::new(),
            }
        }

        fn pathless() -> Self {
            let mut repo = ptr::null_mut();
            // SAFETY: a writable slot.
            assert_eq!(unsafe { ffi::git_repository_new(&mut repo) }, 0);
            Self {
                repo,
                held: Vec::new(),
            }
        }

        /// The step's outcome, and the index of the earlier result it is the same tag
        /// as; a found tag is held.
        fn run(&mut self, step: Step<'_>) -> (Outcome, Option<usize>) {
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id, and `hex.len()` readable bytes of hex digits.
            let rc = unsafe {
                ffi::git_oid_from_prefix(
                    &mut id,
                    step.hex.as_ptr().cast(),
                    step.hex.len(),
                    ffi::GIT_OID_SHA1,
                )
            };
            assert_eq!(rc, 0);
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, the live repository, and a live id of its type
            // read for the call.
            let rc = unsafe {
                match step.len {
                    None => ffi::git_tag_lookup(&mut out, self.repo, &id),
                    Some(len) => ffi::git_tag_lookup_prefix(&mut out, self.repo, &id, len),
                }
            };
            if rc < 0 {
                let (class, message) = c_last();
                return (Outcome::Failed(rc, class, message), None);
            }
            let same = self.held.iter().position(|&h| h == out);
            self.held.push(out);
            (found(out), same)
        }

        /// Copies every held tag (`git_tag_dup`), releases the originals, and views the
        /// copies.
        fn copy_and_release(&mut self) -> Vec<Outcome> {
            let copies: Vec<_> = self
                .held
                .iter()
                .map(|&t| {
                    let mut out = ptr::null_mut();
                    // SAFETY: `t` is a live reference this side holds.
                    assert_eq!(unsafe { ffi::git_tag_dup(&mut out, t) }, 0);
                    assert_eq!(out, t);
                    out
                })
                .collect();
            for t in std::mem::replace(&mut self.held, copies) {
                // SAFETY: each original reference is released once; its copy remains.
                unsafe { ffi::git_tag_free(t) };
            }
            self.held.iter().map(|&t| found(t)).collect()
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            // SAFETY: every held reference is released once, then the repository.
            unsafe {
                for &t in &self.held {
                    ffi::git_tag_free(t);
                }
                ffi::git_repository_free(self.repo);
            }
        }
    }

    /// C's view of what a wrapper gave.
    fn outcome(result: &Result<TagArc<'_>, RepositoryError>) -> Outcome {
        match result {
            Ok(tag) => found(tag.as_c_ptr()),
            Err(e) => Outcome::Failed(
                e.raw_code(),
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            ),
        }
    }

    /// The Rust side's step: the outcome, and the earlier result it is the same tag as.
    fn run_rust<'r>(
        repo: &'r RepositoryBox,
        held: &mut Vec<TagArc<'r>>,
        step: Step<'_>,
    ) -> (Outcome, Option<usize>) {
        let id = Oid::from_prefix(step.hex.as_bytes(), OidType::Sha1).unwrap();
        let result = match step.len {
            None => Tag::lookup(repo.as_ref(), id.as_ref()),
            Some(len) => Tag::lookup_prefix(repo.as_ref(), id.as_ref(), len),
        };
        let seen = outcome(&result);
        let Ok(tag) = result else {
            return (seen, None);
        };
        let same = held.iter().position(|h| TagArc::ptr_eq(h, &tag));
        held.push(tag);
        (seen, same)
    }

    fn script() -> Vec<Step<'static>> {
        let mut steps = Vec::new();
        for hex in TAGS.into_iter().chain([COMMIT, TREE, BLOB, MISSING, ZERO]) {
            steps.push(Step { hex, len: None });
            steps.push(Step { hex, len: Some(9) });
        }
        for len in [0, 1, 3, 4, 5, 7, 8, 20, 39, 40, 41, 64, usize::MAX] {
            steps.push(Step {
                hex: TAGS[0],
                len: Some(len),
            });
        }
        // Two trees start `1810`, a commit and a blob `3d35`; the others start one tag.
        for hex in [
            "1810", "3d35", "b25f", "521d8", "4a23e", "7b438", "849a", "ffff",
        ] {
            steps.push(Step {
                hex,
                len: Some(hex.len()),
            });
        }
        steps.push(Step {
            hex: TAGS[0],
            len: None,
        });
        steps
    }

    /// Runs `script` on both sides, comparing after each step and after the copies; the
    /// number of tags the script found.
    fn compare(mut c: CSide, rust: &RepositoryBox, script: &[Step<'_>]) -> usize {
        let mut held = Vec::new();
        for &step in script {
            assert_eq!(c.run(step), run_rust(rust, &mut held, step), "{step:?}");
        }
        let c_copies = c.copy_and_release();
        let copies: Vec<_> = held
            .iter()
            .enumerate()
            .map(|(i, t)| match i % 3 {
                0 => t.clone(),
                1 => t.as_ref().to_tag(),
                _ => Tag::from_object(Tag::into_object(t.clone())).unwrap(),
            })
            .collect();
        for (copy, original) in copies.iter().zip(&held) {
            assert!(TagArc::ptr_eq(copy, original));
        }
        drop(held);
        let rust_copies: Vec<_> = copies.iter().map(|t| found(t.as_c_ptr())).collect();
        assert_eq!(c_copies, rust_copies);
        let found = copies.len();

        // The same lookups again, with only the copies alive.
        let mut held = copies;
        for &step in script {
            assert_eq!(
                c.run(step),
                run_rust(rust, &mut held, step),
                "again: {step:?}"
            );
        }
        found
    }

    #[test]
    fn lookups_match_c() {
        init();
        let rust = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        assert_eq!(compare(CSide::open(TESTREPO), &rust, &script()), 26);
    }

    #[test]
    fn lookups_in_a_repository_without_paths_match_c() {
        init();
        let rust = Repository::new().unwrap();
        let steps = [
            Step {
                hex: TAGS[0],
                len: None,
            },
            Step {
                hex: TAGS[0],
                len: Some(7),
            },
            Step {
                hex: TAGS[0],
                len: Some(3),
            },
        ];
        assert_eq!(compare(CSide::pathless(), &rust, &steps), 0);
    }

    /// A repository holding tags libgit2 cannot parse, beside well-formed ones (whose
    /// targets it lacks, so their peel fails alike): each lookup fails, or succeeds,
    /// alike.
    #[test]
    fn lookups_of_unparsable_tags_match_c() {
        init();
        let path = std::env::temp_dir().join(format!("crustify-tag-equiv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let path_str = path.to_str().unwrap().to_owned();
        let target = format!("object {COMMIT}\ntype commit\n");
        let objects: Vec<(String, ffi::git_object_t)> = vec![
            ("object 1234\n".to_owned(), ffi::GIT_OBJECT_TAG),
            (
                format!("object {COMMIT}\ntype nope\ntag x\n"),
                ffi::GIT_OBJECT_TAG,
            ),
            (format!("{target}tag no-newline"), ffi::GIT_OBJECT_TAG),
            (
                format!("{target}tag x\ntagger nobody\n\nm\n"),
                ffi::GIT_OBJECT_TAG,
            ),
            (
                format!("{target}tag x\ntagger A <a@b> 0 +0000\nno blank"),
                ffi::GIT_OBJECT_TAG,
            ),
            (format!("{target}tag bare\n"), ffi::GIT_OBJECT_TAG),
            (
                format!("{target}tag full\ntagger A <a@b> 0 +0000\n\nm\n"),
                ffi::GIT_OBJECT_TAG,
            ),
            (format!("object {COMMIT}\n"), ffi::GIT_OBJECT_TAG),
            (
                format!("object {COMMIT}\nkind commit\ntag x\n"),
                ffi::GIT_OBJECT_TAG,
            ),
            (format!("object {COMMIT}\ntype comm"), ffi::GIT_OBJECT_TAG),
            (format!("{target}ta"), ffi::GIT_OBJECT_TAG),
            (format!("{target}name x\n"), ffi::GIT_OBJECT_TAG),
            (
                format!("{target}tag more\ntagger A <a@b> 0 +0000\nextra x\n\nm\n"),
                ffi::GIT_OBJECT_TAG,
            ),
            ("fine\n".to_owned(), ffi::GIT_OBJECT_BLOB),
        ];
        let ids: Vec<String> = {
            let repo = Repository::init(&CString::new(path_str.as_str()).unwrap(), true).unwrap();
            let odb = repo.as_ref().odb().unwrap();
            objects
                .iter()
                .map(|(data, kind)| {
                    let mut id = ffi::git_oid::default();
                    // SAFETY: a live odb, a writable id, `data` read for the call.
                    let rc = unsafe {
                        ffi::git_odb_write(
                            &mut id,
                            odb.as_c_ptr(),
                            data.as_ptr().cast(),
                            data.len(),
                            *kind,
                        )
                    };
                    assert_eq!(rc, 0);
                    hex_of(&id)
                })
                .collect()
        };
        let mut steps = Vec::new();
        for hex in &ids {
            steps.push(Step { hex, len: None });
            steps.push(Step { hex, len: Some(12) });
        }
        let rust = Repository::open(&CString::new(path_str.as_str()).unwrap()).unwrap();
        assert_eq!(compare(CSide::open(&path_str), &rust, &steps), 6);
        drop(rust);
        let _ = std::fs::remove_dir_all(&path);
    }
}

mod repository_tags {
    //! Equivalence of [`create_lightweight`], [`create_from_buffer`], [`delete`],
    //! [`list`] and [`list_match`] with `git_tag_create_lightweight`,
    //! `git_tag_create_from_buffer`, `git_tag_delete`, `git_tag_list` and
    //! `git_tag_list_match`.
    //!
    //! Each side works on a copy of `testrepo.git` of its own and runs one script: tags
    //! created over every kind of target, again without and with `force`, under names
    //! the tag rules or the reference rules refuse, with a target of another repository
    //! (opened by each side on its own); annotated tags from buffers that are
    //! well-formed, that name a missing target or a target of another type, that are
    //! malformed in their first line, or that name an invalid reference; deletions of
    //! tags that exist, are packed, are gone, or have invalid names; and listings under
    //! a range of patterns. After every step the sides compare the outcome (the id
    //! written or the names listed, or the code, error class and message) and every
    //! tag's name and target as each side lists them afresh.
    //!
    //! Buffers recording a name that is no valid reference name, on which C leaks the
    //! reference name it built, are compared in [`invalid_reference_names_match_c`],
    //! whose child process runs with LeakSanitizer off: alone, and behind a missing
    //! target or one of another type, which C reports first.
    //!
    //! Documented divergences, outside these comparisons, held by `tag.rs`'s unit tests:
    //!
    //! - A buffer whose target is the zero id is refused before C, which frees an
    //!   uninitialised pointer for it; a buffer malformed after the tag's name is
    //!   refused by a parse that frees what it allocated, where C leaks the name and
    //!   tagger (with the same error and code). An invalid reference name is refused
    //!   in Rust too, with C's outcome, but compared with C's (above).
    //! - A listing that fails is an error (code `-1`, with the error libgit2 set), where
    //!   C returns 0 and an empty array, leaking the names it collected.

    use std::ffi::{CStr, CString, c_int};
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tag::{
        create_from_buffer, create_lightweight, delete, list, list_match,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    use super::{c_last, init};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git"
    );
    const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    const BLOB: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
    const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
    const MISSING: &str = "dededededededededededededededededededede";

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// A private copy of `testrepo.git`, removed on drop.
    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("crustify-tag-equiv-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            copy_dir(Path::new(TESTREPO), &path);
            Self(path)
        }

        fn c_path(&self) -> CString {
            CString::new(self.0.to_str().unwrap()).unwrap()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An annotated tag's content: `name`, of `target`, recorded as of type `kind`.
    fn annotated(target: &str, kind: &str, name: &str) -> String {
        format!(
            "object {target}\ntype {kind}\ntag {name}\ntagger T <t@example.com> 1 +0000\n\nmessage\n"
        )
    }

    /// One step of the script.
    #[derive(Clone, Debug)]
    enum Step {
        List,
        Match(&'static str),
        Lightweight(&'static str, &'static str, bool),
        Foreign(&'static str),
        Buffer(String, bool),
        Delete(&'static str),
    }

    fn script() -> Vec<Step> {
        use Step::*;
        let mut steps = vec![List, Match(""), Match("*")];
        for (name, target) in [
            ("light", COMMIT),
            ("to-tree", TREE),
            ("to-blob", BLOB),
            ("to-tag", TAG),
        ] {
            steps.push(Lightweight(name, target, false));
        }
        steps.extend([
            Lightweight("light", TREE, false),
            Lightweight("light", TREE, true),
            Lightweight("test", COMMIT, false),
            Lightweight("-dash", COMMIT, false),
            Lightweight("HEAD", COMMIT, false),
            Lightweight("a..b", COMMIT, true),
            Lightweight("", COMMIT, false),
            Lightweight("dir/nested", COMMIT, false),
            Lightweight("dir", COMMIT, false),
            Foreign("foreign"),
            Buffer(annotated(COMMIT, "commit", "v2.0"), false),
            Buffer(annotated(COMMIT, "commit", "v2.0"), false),
            Buffer(annotated(COMMIT, "commit", "v2.0"), true),
            Buffer(annotated(TREE, "tree", "v2.1"), false),
            Buffer(annotated(TAG, "tag", "nested"), false),
            Buffer(annotated(MISSING, "commit", "missing"), false),
            Buffer(annotated(COMMIT, "tree", "mistyped"), false),
            Buffer(annotated(COMMIT, "commit", "-dash"), false),
            Buffer(annotated(COMMIT, "commit", "/lead"), false),
            Buffer(annotated(COMMIT, "commit", "//lead"), true),
            Buffer(
                format!("object {COMMIT}\ntype commit\ntag minimal\n"),
                false,
            ),
            Buffer(String::new(), false),
            Buffer("object nope\n".into(), false),
            Buffer(format!("object {COMMIT}\nkind commit\n"), false),
        ]);
        for pattern in [
            "", "*", "v*", "t*", "*tag*", "[a-e]*", "\\*", "dir/*", "dir", "nomatch", "e90810b",
            "?ight",
        ] {
            steps.push(Match(pattern));
        }
        steps.extend([
            Delete("light"),
            Delete("light"),
            Delete("test"),
            Delete("a..b"),
            Delete(""),
            Delete("dir/nested"),
            Delete("v2.0"),
            List,
        ]);
        steps
    }

    /// What a step gave.
    #[derive(Debug, PartialEq, Eq)]
    enum Outcome {
        Id(String),
        Names(Vec<String>),
        Done,
        Failed(c_int, c_int, String),
    }

    fn hex(id: &ffi::git_oid) -> String {
        let mut out = [0u8; 65];
        // SAFETY: 65 writable bytes bound the write; `id` is live.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn c_failed(rc: c_int) -> Outcome {
        let (class, message) = c_last();
        Outcome::Failed(rc, class, message)
    }

    fn rust_failed(e: &RepositoryError) -> Outcome {
        Outcome::Failed(
            e.raw_code(),
            e.raw_class(),
            e.message().to_string_lossy().into_owned(),
        )
    }

    /// The C side: its repository, and the one its foreign targets come from.
    struct CSide {
        repo: *mut ffi::git_repository,
        other: *mut ffi::git_repository,
    }

    impl CSide {
        fn open(path: &CStr) -> Self {
            let open = |path: &CStr| {
                let mut repo = ptr::null_mut();
                // SAFETY: a writable slot and a NUL-terminated path.
                let rc = unsafe { ffi::git_repository_open(&mut repo, path.as_ptr()) };
                assert_eq!(rc, 0);
                repo
            };
            let other = CString::new(TESTREPO).unwrap();
            Self {
                repo: open(path),
                other: open(&other),
            }
        }

        fn lookup(repo: *mut ffi::git_repository, target: &str) -> *mut ffi::git_object {
            let mut id = ffi::git_oid::default();
            let target = CString::new(target).unwrap();
            let mut out = ptr::null_mut();
            // SAFETY: writable slots, a hex id and a live repository.
            unsafe {
                assert_eq!(
                    ffi::git_oid_from_string(&mut id, target.as_ptr(), ffi::GIT_OID_SHA1),
                    0
                );
                assert_eq!(
                    ffi::git_object_lookup(&mut out, repo, &id, ffi::GIT_OBJECT_ANY),
                    0
                );
            }
            out
        }

        fn list(&self, pattern: Option<&str>) -> Outcome {
            let mut names = ffi::git_strarray {
                strings: ptr::null_mut(),
                count: 0,
            };
            let pattern = pattern.map(|p| CString::new(p).unwrap());
            // SAFETY: an empty array libgit2 fills; the live repository; a NUL-terminated
            // pattern. The array is read, then disposed once.
            unsafe {
                let rc = match &pattern {
                    None => ffi::git_tag_list(&mut names, self.repo),
                    Some(p) => ffi::git_tag_list_match(&mut names, p.as_ptr(), self.repo),
                };
                if rc < 0 {
                    return c_failed(rc);
                }
                assert!(
                    !names.strings.is_null(),
                    "C dropped its listing: {:?}",
                    c_last()
                );
                let listed = (0..names.count)
                    .map(|i| {
                        CStr::from_ptr(*names.strings.add(i))
                            .to_str()
                            .unwrap()
                            .to_owned()
                    })
                    .collect();
                ffi::git_strarray_dispose(&mut names);
                Outcome::Names(listed)
            }
        }

        /// `git_tag_create_lightweight` of `name` at `object`, which it releases after.
        fn lightweight(&self, name: &str, object: *mut ffi::git_object, force: bool) -> Outcome {
            let mut id = ffi::git_oid::default();
            let name = CString::new(name).unwrap();
            // SAFETY: a writable id, the live repository, a NUL-terminated name and a live
            // object, released once after the call.
            let rc = unsafe {
                let rc = ffi::git_tag_create_lightweight(
                    &mut id,
                    self.repo,
                    name.as_ptr(),
                    object,
                    c_int::from(force),
                );
                ffi::git_object_free(object);
                rc
            };
            if rc < 0 {
                c_failed(rc)
            } else {
                Outcome::Id(hex(&id))
            }
        }

        fn run(&mut self, step: &Step) -> Outcome {
            match step {
                Step::List => self.list(None),
                Step::Match(pattern) => self.list(Some(pattern)),
                Step::Lightweight(name, target, force) => {
                    let object = Self::lookup(self.repo, target);
                    self.lightweight(name, object, *force)
                }
                Step::Foreign(name) => {
                    let object = Self::lookup(self.other, COMMIT);
                    self.lightweight(name, object, false)
                }
                Step::Buffer(buffer, force) => {
                    let mut id = ffi::git_oid::default();
                    let buffer = CString::new(buffer.as_str()).unwrap();
                    // SAFETY: a writable id, the live repository and a NUL-terminated
                    // buffer whose target is not the zero id, malformed (if at all) before
                    // C allocates anything.
                    let rc = unsafe {
                        ffi::git_tag_create_from_buffer(
                            &mut id,
                            self.repo,
                            buffer.as_ptr(),
                            c_int::from(*force),
                        )
                    };
                    if rc < 0 {
                        c_failed(rc)
                    } else {
                        Outcome::Id(hex(&id))
                    }
                }
                Step::Delete(name) => {
                    let name = CString::new(*name).unwrap();
                    // SAFETY: the live repository and a NUL-terminated name.
                    let rc = unsafe { ffi::git_tag_delete(self.repo, name.as_ptr()) };
                    if rc < 0 { c_failed(rc) } else { Outcome::Done }
                }
            }
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            // SAFETY: both repositories are released once; nothing of them is held.
            unsafe {
                ffi::git_repository_free(self.repo);
                ffi::git_repository_free(self.other);
            }
        }
    }

    fn rust_lookup<'r>(repo: &'r RepositoryBox, target: &str) -> ObjectArc<'r> {
        let id = Oid::from_string(&CString::new(target).unwrap(), OidType::Sha1).unwrap();
        Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap()
    }

    fn rust_list(
        result: Result<libgit2::libgit2::core::strarray::StrArrayVal, RepositoryError>,
    ) -> Outcome {
        match result {
            Ok(names) => Outcome::Names(
                names
                    .as_ref()
                    .iter()
                    .map(|n| n.to_str().unwrap().to_owned())
                    .collect(),
            ),
            Err(e) => rust_failed(&e),
        }
    }

    fn rust_run(repo: &RepositoryBox, other: &RepositoryBox, step: &Step) -> Outcome {
        let created = |result: Result<Oid, RepositoryError>| match result {
            Ok(id) => Outcome::Id(id.as_ref().to_hex_string().unwrap()),
            Err(e) => rust_failed(&e),
        };
        match step {
            Step::List => rust_list(list(repo.as_ref())),
            Step::Match(pattern) => {
                rust_list(list_match(repo.as_ref(), &CString::new(*pattern).unwrap()))
            }
            Step::Lightweight(name, target, force) => {
                let object = rust_lookup(repo, target);
                let name = CString::new(*name).unwrap();
                created(create_lightweight(
                    repo.as_ref(),
                    &name,
                    object.as_ref(),
                    *force,
                ))
            }
            Step::Foreign(name) => {
                let object = rust_lookup(other, COMMIT);
                let name = CString::new(*name).unwrap();
                created(create_lightweight(
                    repo.as_ref(),
                    &name,
                    object.as_ref(),
                    false,
                ))
            }
            Step::Buffer(buffer, force) => {
                let buffer = CString::new(buffer.as_str()).unwrap();
                created(create_from_buffer(repo.as_ref(), &buffer, *force))
            }
            Step::Delete(name) => match delete(repo.as_ref(), &CString::new(*name).unwrap()) {
                Ok(()) => Outcome::Done,
                Err(e) => rust_failed(&e),
            },
        }
    }

    /// Every tag's name and the id its reference resolves to, as C reads a side's
    /// repository.
    fn tags(repo: *mut ffi::git_repository) -> Vec<(String, String)> {
        let mut names = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 0,
        };
        // SAFETY: an empty array libgit2 fills, read and disposed once; the live
        // repository; each reference name is NUL-terminated and `id` writable.
        unsafe {
            assert_eq!(ffi::git_tag_list(&mut names, repo), 0);
            let tags = (0..names.count)
                .map(|i| {
                    let name = CStr::from_ptr(*names.strings.add(i))
                        .to_str()
                        .unwrap()
                        .to_owned();
                    let refname = CString::new(format!("refs/tags/{name}")).unwrap();
                    let mut reference = ptr::null_mut();
                    assert_eq!(
                        ffi::git_reference_lookup(&mut reference, repo, refname.as_ptr()),
                        0
                    );
                    let target = ffi::git_reference_target(reference);
                    let id = if target.is_null() {
                        String::new()
                    } else {
                        hex(&*target)
                    };
                    ffi::git_reference_free(reference);
                    (name, id)
                })
                .collect();
            ffi::git_strarray_dispose(&mut names);
            tags
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn scripts_match_c() {
        isolated(concat!(module_path!(), "::scripts_match_c"), false, || {
            init();
            let (c_box, rust_box) = (Sandbox::new("c"), Sandbox::new("rust"));
            let mut c = CSide::open(&c_box.c_path());
            let rust_repo = Repository::open(&rust_box.c_path()).unwrap();
            let rust_other = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
            let mut failures = 0;
            for (i, step) in script().iter().enumerate() {
                let c_seen = c.run(step);
                let rust_seen = rust_run(&rust_repo, &rust_other, step);
                assert_eq!(rust_seen, c_seen, "step {i}: {step:?}");
                failures += usize::from(matches!(c_seen, Outcome::Failed(..)));
                assert_eq!(
                    tags(rust_repo.as_c_ptr()),
                    tags(c.repo),
                    "after step {i}: {step:?}"
                );
            }
            assert!(failures > 10, "{failures}");
            let names = rust_list(list(rust_repo.as_ref()));
            let Outcome::Names(names) = names else {
                panic!("{names:?}")
            };
            assert!(names.contains(&"to-tag".to_owned()) && !names.contains(&"light".to_owned()));
        });
    }

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process: writing a reference reads the time through
    /// glibc's `tzset`, which TSan reports against any concurrent caller. With
    /// `tolerate_leaks`, LeakSanitizer is off there: C leaks.
    fn isolated(test: &str, tolerate_leaks: bool, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name);
        if tolerate_leaks {
            let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            let separator = if inherited.is_empty() { "" } else { ":" };
            command.env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            );
        }
        let out = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn invalid_reference_names_match_c() {
        isolated(
            concat!(module_path!(), "::invalid_reference_names_match_c"),
            true,
            || {
                init();
                let (c_box, rust_box) = (Sandbox::new("names-c"), Sandbox::new("names-rust"));
                let mut c = CSide::open(&c_box.c_path());
                let rust_repo = Repository::open(&rust_box.c_path()).unwrap();
                let rust_other = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
                let long = "n".repeat(1100);
                let steps = [
                    Step::Buffer(annotated(COMMIT, "commit", "bad..name"), false),
                    Step::Buffer(annotated(COMMIT, "commit", "bad..name"), true),
                    Step::Buffer(annotated(COMMIT, "commit", "x.lock"), false),
                    Step::Buffer(annotated(COMMIT, "commit", "a b"), false),
                    Step::Buffer(annotated(COMMIT, "commit", "end/"), false),
                    Step::Buffer(annotated(COMMIT, "commit", &long), false),
                    Step::Buffer(annotated(MISSING, "commit", "bad..name"), false),
                    Step::Buffer(annotated(COMMIT, "tree", "bad..name"), false),
                    Step::Buffer(annotated(TREE, "tree", "bad..name"), false),
                    Step::Buffer(annotated(COMMIT, "commit", "fine"), false),
                ];
                for (i, step) in steps.iter().enumerate() {
                    let c_seen = c.run(step);
                    let rust_seen = rust_run(&rust_repo, &rust_other, step);
                    assert_eq!(rust_seen, c_seen, "step {i}");
                    assert_eq!(tags(rust_repo.as_c_ptr()), tags(c.repo), "after step {i}");
                }
            },
        );
    }
}
