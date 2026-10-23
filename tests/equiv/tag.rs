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
        "/c/tests/resources/testrepo.git"
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

    use std::num::NonZero;
    use std::ops::ControlFlow;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{SignatureView, Time};
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tag::{
        ForeachCallback, Tag, create, create_annotation, create_from_buffer, create_lightweight,
        delete, foreach, list, list_match,
    };
    use libgit2::libgit2::oid::oid::{Oid, OidRef, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};
    use libgit2::libgit2::util::errors::last;

    use super::{c_last, init};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git"
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
        /// `git_tag_create`: name, target, tagger, message, force.
        Annotated(&'static str, &'static str, Tagger, &'static str, bool),
        /// `git_tag_annotation_create`: name, target, tagger, message.
        Annotation(&'static str, &'static str, Tagger, &'static str),
        /// Either writer, with a target of another repository: name, `force` (or
        /// `None` for `git_tag_annotation_create`).
        ForeignAnnotated(&'static str, Option<bool>),
        /// `git_tag_foreach`, its callback breaking with the code at the given call.
        Foreach(Option<(usize, c_int)>),
        /// A symbolic reference `refs/tags/<name>` to a missing branch, written into
        /// each side's repository.
        Dangle(&'static str),
    }

    /// The tagger a writer is given.
    #[derive(Clone, Copy, Debug)]
    enum Tagger {
        /// A signature over borrowed strings: name, email, seconds, offset.
        View(&'static str, &'static str, i64, i32),
        /// The tagger of the tag object `TAG`, borrowed from the tag libgit2 parsed.
        Parsed,
    }

    const TAGGER: Tagger = Tagger::View("A U Thor", "author@example.com", 1_700_000_000, 90);

    fn annotated_script() -> Vec<Step> {
        use Step::*;
        let lisa = Tagger::View("Lisa", "lisa@example.com", 0, -420);
        vec![
            Foreach(None),
            Annotated("ann", COMMIT, TAGGER, "first\n", false),
            Annotated("ann", COMMIT, TAGGER, "first\n", false),
            Annotated("ann", TREE, lisa, "replaced\n", true),
            Annotated("to-blob", BLOB, TAGGER, "", false),
            Annotated("to-tag", TAG, Tagger::Parsed, "no newline", false),
            Annotated("multi", COMMIT, TAGGER, "line 1\n\nline 3\n", false),
            Annotated("-dash", COMMIT, TAGGER, "m\n", false),
            Annotated("HEAD", COMMIT, TAGGER, "m\n", false),
            Annotated("bad..name", COMMIT, TAGGER, "m\n", false),
            Annotated("trailing/", COMMIT, TAGGER, "m\n", false),
            Annotated("nested/deep/name", COMMIT, lisa, "m\n", false),
            Annotated("test", COMMIT, TAGGER, "m\n", false),
            Annotated("test", COMMIT, TAGGER, "replaces test\n", true),
            ForeignAnnotated("foreign", Some(false)),
            ForeignAnnotated("foreign", None),
            Annotation("ann", COMMIT, TAGGER, "first\n"),
            Annotation("-dash", BLOB, lisa, "any name\n"),
            Annotation("bad..name", TREE, Tagger::Parsed, ""),
            Annotation("new\nline", COMMIT, TAGGER, "m\n"),
            Foreach(None),
            Foreach(Some((0, 1))),
            Foreach(Some((3, -1))),
            Foreach(Some((5, -12345))),
            Foreach(Some((100, 7))),
            Dangle("zz-dangling"),
            Foreach(None),
            Foreach(Some((1, 9))),
            Delete("zz-dangling"),
            Foreach(None),
            Delete("ann"),
            Delete("test"),
            Foreach(Some((2, 2))),
            List,
        ]
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
        /// A tag object written: its id and raw content, as C reads it from the side's
        /// object database.
        Object(String, Vec<u8>),
        /// A walk: every tag seen (full name and id), what it returned, and, when that
        /// was not 0, the error left.
        Walk(Vec<(String, String)>, c_int, Option<(c_int, String)>),
    }

    /// The tag object `id` names in `repo`, as an outcome.
    fn object(repo: *mut ffi::git_repository, id: &ffi::git_oid) -> Outcome {
        let mut odb = ptr::null_mut();
        let mut object = ptr::null_mut();
        // SAFETY: writable slots, the live repository and a live id; the object's data
        // is copied at once, then the object and the odb are released once.
        unsafe {
            assert_eq!(ffi::git_repository_odb(&mut odb, repo), 0);
            assert_eq!(ffi::git_odb_read(&mut object, odb, id), 0);
            assert_eq!(ffi::git_odb_object_type(object), ffi::GIT_OBJECT_TAG);
            let data = std::slice::from_raw_parts(
                ffi::git_odb_object_data(object).cast::<u8>(),
                ffi::git_odb_object_size(object),
            )
            .to_vec();
            ffi::git_odb_object_free(object);
            ffi::git_odb_free(odb);
            Outcome::Object(hex(id), data)
        }
    }

    /// What `git_tag_foreach`'s C callback collects, and when it breaks.
    struct Collect {
        seen: Vec<(String, String)>,
        stop: Option<(usize, c_int)>,
    }

    unsafe extern "C" fn collect(
        name: *const std::ffi::c_char,
        id: *mut ffi::git_oid,
        payload: *mut std::ffi::c_void,
    ) -> c_int {
        // SAFETY: libgit2 passes the walk's `Collect`, a NUL-terminated name and a live
        // id, all for the call.
        let collect = unsafe { &mut *payload.cast::<Collect>() };
        // SAFETY: as above.
        let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap().to_owned();
        // SAFETY: as above.
        collect.seen.push((name, hex(unsafe { &*id })));
        match collect.stop {
            Some((at, code)) if collect.seen.len() == at + 1 => code,
            _ => 0,
        }
    }

    /// C's signature for `tagger`, over `strings`, which must outlive it.
    fn c_signature(tagger: Tagger, strings: &(CString, CString)) -> ffi::git_signature {
        let Tagger::View(_, _, seconds, offset) = tagger else {
            unreachable!("a parsed tagger is the tag's own")
        };
        ffi::git_signature {
            name: strings.0.as_ptr().cast_mut(),
            email: strings.1.as_ptr().cast_mut(),
            when: ffi::git_time {
                time: seconds,
                offset,
                sign: if offset < 0 { b'-' } else { b'+' } as std::ffi::c_char,
            },
        }
    }

    fn tagger_strings(tagger: Tagger) -> (CString, CString) {
        match tagger {
            Tagger::View(name, email, ..) => {
                (CString::new(name).unwrap(), CString::new(email).unwrap())
            }
            Tagger::Parsed => (CString::default(), CString::default()),
        }
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

    fn raw_id(hex: &str) -> ffi::git_oid {
        let mut id = ffi::git_oid::default();
        let hex = CString::new(hex).unwrap();
        // SAFETY: a writable id and a NUL-terminated hex string.
        let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        id
    }

    fn object_of(repo: *mut ffi::git_repository, id: &ffi::git_oid) -> Outcome {
        object(repo, id)
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

        /// `git_tag_create` (with `force`) or `git_tag_annotation_create` (without) of
        /// `name` at `object`, which it releases after.
        fn annotated(
            &self,
            name: &str,
            object: *mut ffi::git_object,
            tagger: Tagger,
            message: &str,
            force: Option<bool>,
        ) -> Outcome {
            let (name, message) = (CString::new(name).unwrap(), CString::new(message).unwrap());
            let strings = tagger_strings(tagger);
            let mut tag = ptr::null_mut();
            let signature = match tagger {
                Tagger::View(..) => c_signature(tagger, &strings),
                Tagger::Parsed => {
                    let id = raw_id(TAG);
                    // SAFETY: a writable slot, the live repository and a live id; the tag
                    // is released below, after its tagger was copied out.
                    unsafe {
                        assert_eq!(ffi::git_tag_lookup(&mut tag, self.repo, &id), 0);
                        *ffi::git_tag_tagger(tag)
                    }
                }
            };
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id, the live repository, NUL-terminated strings and a
            // live object, released once after; the signature's strings are `strings`
            // or the live tag's, both outliving the call.
            let rc = unsafe {
                let rc = match force {
                    Some(force) => ffi::git_tag_create(
                        &mut id,
                        self.repo,
                        name.as_ptr(),
                        object,
                        &signature,
                        message.as_ptr(),
                        c_int::from(force),
                    ),
                    None => ffi::git_tag_annotation_create(
                        &mut id,
                        self.repo,
                        name.as_ptr(),
                        object,
                        &signature,
                        message.as_ptr(),
                    ),
                };
                ffi::git_object_free(object);
                ffi::git_tag_free(tag);
                rc
            };
            if rc < 0 {
                c_failed(rc)
            } else {
                object_of(self.repo, &id)
            }
        }

        fn walk(&self, stop: Option<(usize, c_int)>) -> Outcome {
            let mut collected = Collect {
                seen: Vec::new(),
                stop,
            };
            // SAFETY: the live repository; `collect` reads the payload it is given, the
            // live `Collect`, for the walk only.
            let rc = unsafe {
                ffi::git_tag_foreach(self.repo, Some(collect), (&raw mut collected).cast())
            };
            let error = (rc != 0).then(c_last);
            Outcome::Walk(collected.seen, rc, error)
        }

        fn run(&mut self, step: &Step) -> Outcome {
            match step {
                Step::Annotated(name, target, tagger, message, force) => {
                    let object = Self::lookup(self.repo, target);
                    self.annotated(name, object, *tagger, message, Some(*force))
                }
                Step::Annotation(name, target, tagger, message) => {
                    let object = Self::lookup(self.repo, target);
                    self.annotated(name, object, *tagger, message, None)
                }
                Step::ForeignAnnotated(name, force) => {
                    let object = Self::lookup(self.other, COMMIT);
                    self.annotated(name, object, TAGGER, "foreign\n", *force)
                }
                Step::Foreach(stop) => self.walk(*stop),
                Step::Dangle(_) => unreachable!("written by the test"),
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
        let tagged = |result: Result<Oid, RepositoryError>| match result {
            Ok(id) => {
                let raw = ffi::git_oid {
                    type_: id.as_ref().raw_kind(),
                    id: id.as_ref().id(),
                };
                object_of(repo.as_c_ptr(), &raw)
            }
            Err(e) => rust_failed(&e),
        };
        let write = |name: &str, object: ObjectArc<'_>, tagger: Tagger, message: &str, force| {
            let (name, message) = (CString::new(name).unwrap(), CString::new(message).unwrap());
            let strings = tagger_strings(tagger);
            let tag;
            let view;
            let signature = match tagger {
                Tagger::View(_, _, seconds, offset) => {
                    view = SignatureView::new(&strings.0, &strings.1, Time::new(seconds, offset));
                    view.as_ref()
                }
                Tagger::Parsed => {
                    let id = Oid::from_string(&CString::new(TAG).unwrap(), OidType::Sha1).unwrap();
                    tag = Tag::lookup(repo.as_ref(), id.as_ref()).unwrap();
                    tag.as_ref().tagger().unwrap()
                }
            };
            tagged(match force {
                Some(force) => create(
                    repo.as_ref(),
                    &name,
                    object.as_ref(),
                    signature,
                    &message,
                    force,
                ),
                None => {
                    create_annotation(repo.as_ref(), &name, object.as_ref(), signature, &message)
                }
            })
        };
        match step {
            Step::Annotated(name, target, tagger, message, force) => write(
                name,
                rust_lookup(repo, target),
                *tagger,
                message,
                Some(*force),
            ),
            Step::Annotation(name, target, tagger, message) => {
                write(name, rust_lookup(repo, target), *tagger, message, None)
            }
            Step::ForeignAnnotated(name, force) => write(
                name,
                rust_lookup(other, COMMIT),
                TAGGER,
                "foreign\n",
                *force,
            ),
            Step::Foreach(stop) => {
                let mut seen = Vec::new();
                let mut f = |name: &CStr, id: OidRef<'_>| {
                    seen.push((
                        name.to_str().unwrap().to_owned(),
                        id.to_hex_string().unwrap(),
                    ));
                    match stop {
                        Some((at, code)) if seen.len() == at + 1 => {
                            ControlFlow::Break(NonZero::new(*code).unwrap())
                        }
                        _ => ControlFlow::Continue(()),
                    }
                };
                let mut callback = ForeachCallback::new(&mut f);
                let result = foreach(repo.as_ref(), &mut callback);
                drop(callback);
                let (rc, error) = match result {
                    Ok(ControlFlow::Continue(())) => (0, None),
                    Ok(ControlFlow::Break(code)) => {
                        let e = last();
                        let message = e.message().to_string_lossy().into_owned();
                        (code.get(), Some((e.raw_class(), message)))
                    }
                    Err(e) => (
                        e.raw_code(),
                        Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
                    ),
                };
                Outcome::Walk(seen, rc, error)
            }
            Step::Dangle(_) => unreachable!("written by the test"),
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

    /// [`create`], [`create_annotation`] and [`foreach`] against `git_tag_create`,
    /// `git_tag_annotation_create` and `git_tag_foreach`: annotated tags over every kind
    /// of target, with taggers over borrowed strings and one borrowed from a parsed tag,
    /// again without and with `force`, under names the tag rules or the reference rules
    /// refuse, with a target of another repository; tag objects alone, under any name;
    /// and walks running to the end, breaking with codes of either sign at their first,
    /// middle and past their last tag, and failing on a dangling symbolic reference.
    /// After every step the sides compare the outcome (the tag object written, as its id
    /// and raw content, the tags walked and the code and error a walk returned, or the
    /// code, error class and message) and every tag's name and target.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn annotated_scripts_match_c() {
        isolated(
            concat!(module_path!(), "::annotated_scripts_match_c"),
            false,
            || {
                init();
                let (c_box, rust_box) = (Sandbox::new("ann-c"), Sandbox::new("ann-rust"));
                let mut c = CSide::open(&c_box.c_path());
                let rust_repo = Repository::open(&rust_box.c_path()).unwrap();
                let rust_other = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
                let (mut failures, mut walks) = (0, 0);
                for (i, step) in annotated_script().iter().enumerate() {
                    if let Step::Dangle(name) = step {
                        for sandbox in [&c_box, &rust_box] {
                            let path = sandbox.0.join("refs/tags").join(name);
                            std::fs::write(path, "ref: refs/heads/nope\n").unwrap();
                        }
                        continue;
                    }
                    let c_seen = c.run(step);
                    let rust_seen = rust_run(&rust_repo, &rust_other, step);
                    assert_eq!(rust_seen, c_seen, "step {i}: {step:?}");
                    failures += usize::from(matches!(c_seen, Outcome::Failed(..)));
                    walks += usize::from(matches!(c_seen, Outcome::Walk(_, rc, _) if rc != 0));
                    if !matches!(step, Step::Foreach(..))
                        || matches!(c_seen, Outcome::Walk(_, 0, _))
                    {
                        assert_eq!(
                            tags(rust_repo.as_c_ptr()),
                            tags(c.repo),
                            "after step {i}: {step:?}"
                        );
                    }
                }
                assert!(failures >= 6 && walks >= 6, "{failures} {walks}");
            },
        );
    }

    /// A walk whose callback breaks with `GIT_ITEROVER` (-31) at the first tag: both
    /// sides visit that tag alone and leave the same error ("git_tag_foreach callback
    /// returned -31", unless loading the references left one, such as a missing global
    /// configuration file). Documented divergence: libgit2's reference walk takes -31 for its
    /// own end and returns 0, where [`foreach`] reports the break, `Break(-31)` (held by
    /// `tag.rs`'s unit test `foreach_tells_breaks_from_failures`). Before the fix the
    /// wrapper's debug assertion that libgit2 returned the code panicked here.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn iterover_breaks_match_c_but_for_the_code() {
        init();
        let stop = Some((0, ffi::GIT_ITEROVER));
        let c = CSide::open(&CString::new(TESTREPO).unwrap());
        super::clear();
        let Outcome::Walk(c_seen, c_rc, None) = c.walk(stop) else {
            panic!("C reported the break")
        };
        let c_error = c_last();
        let repo = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        super::clear();
        let mut seen = Vec::new();
        let mut f = |name: &CStr, id: OidRef<'_>| {
            seen.push((
                name.to_str().unwrap().to_owned(),
                id.to_hex_string().unwrap(),
            ));
            ControlFlow::Break(NonZero::new(ffi::GIT_ITEROVER).unwrap())
        };
        let mut callback = ForeachCallback::new(&mut f);
        let walked = foreach(repo.as_ref(), &mut callback);
        drop(callback);
        let e = last();
        let rust_error = (e.raw_class(), e.message().to_string_lossy().into_owned());
        assert_eq!(seen, c_seen);
        assert_eq!(seen.len(), 1);
        assert_eq!(rust_error, c_error);
        assert_eq!(c_rc, 0);
        assert_eq!(
            walked,
            Ok(ControlFlow::Break(NonZero::new(ffi::GIT_ITEROVER).unwrap()))
        );
    }

    /// A tag object written into a repository without an object database (one without
    /// a git directory, its target from another) fails as libgit2 fails it, on both
    /// sides; [`create`] refuses the foreign target first, as libgit2 does.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn annotations_without_an_object_database_match_c() {
        init();
        let other = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        let mut c_repo = ptr::null_mut();
        // SAFETY: a writable slot.
        assert_eq!(unsafe { ffi::git_repository_new(&mut c_repo) }, 0);
        let rust_repo = Repository::new().unwrap();
        let strings = tagger_strings(TAGGER);
        let signature = c_signature(TAGGER, &strings);
        let Tagger::View(_, _, seconds, offset) = TAGGER else {
            unreachable!()
        };
        let view = SignatureView::new(&strings.0, &strings.1, Time::new(seconds, offset));
        for force in [None, Some(false), Some(true)] {
            let object = CSide::lookup(other.as_c_ptr(), COMMIT);
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id, the live pathless repository, NUL-terminated
            // strings, a live signature over `strings`, and a live object of another
            // repository, released once after.
            let rc = unsafe {
                let rc = match force {
                    None => ffi::git_tag_annotation_create(
                        &mut id,
                        c_repo,
                        c"t".as_ptr(),
                        object,
                        &signature,
                        c"m\n".as_ptr(),
                    ),
                    Some(force) => ffi::git_tag_create(
                        &mut id,
                        c_repo,
                        c"t".as_ptr(),
                        object,
                        &signature,
                        c"m\n".as_ptr(),
                        c_int::from(force),
                    ),
                };
                ffi::git_object_free(object);
                rc
            };
            assert!(rc < 0);
            let c_seen = c_failed(rc);
            let target = rust_lookup(&other, COMMIT);
            let result = match force {
                None => create_annotation(
                    rust_repo.as_ref(),
                    c"t",
                    target.as_ref(),
                    view.as_ref(),
                    c"m\n",
                ),
                Some(force) => create(
                    rust_repo.as_ref(),
                    c"t",
                    target.as_ref(),
                    view.as_ref(),
                    c"m\n",
                    force,
                ),
            };
            assert_eq!(rust_failed(&result.unwrap_err()), c_seen, "{force:?}");
        }
        // SAFETY: the raw repository is released once; nothing of it is held.
        unsafe { ffi::git_repository_free(c_repo) };
    }

    /// A tag object the object database fails to store (its loose fan-out directory is
    /// a regular file, which stops root as well) fails both writers alike, and
    /// [`create`] then writes no reference.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn unwritable_objects_match_c() {
        init();
        let scratch = Sandbox::new("unwritable-scratch");
        let scratch_repo = Repository::open(&scratch.c_path()).unwrap();
        let other = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        let used = |hex: &str| Path::new(TESTREPO).join("objects").join(&hex[..2]).exists();
        // A message whose tag object lands in a fan-out the repository does not use.
        const MESSAGES: [&str; 16] = [
            "blocked 0\n",
            "blocked 1\n",
            "blocked 2\n",
            "blocked 3\n",
            "blocked 4\n",
            "blocked 5\n",
            "blocked 6\n",
            "blocked 7\n",
            "blocked 8\n",
            "blocked 9\n",
            "blocked a\n",
            "blocked b\n",
            "blocked c\n",
            "blocked d\n",
            "blocked e\n",
            "blocked f\n",
        ];
        let (message, blocked) = MESSAGES
            .into_iter()
            .map(|message| {
                let step = Step::Annotation("blocked", COMMIT, TAGGER, message);
                let Outcome::Object(hex, _) = rust_run(&scratch_repo, &other, &step) else {
                    panic!("{step:?}")
                };
                (message, hex)
            })
            .find(|(_, hex)| !used(hex))
            .unwrap();
        let (c_box, rust_box) = (
            Sandbox::new("unwritable-c"),
            Sandbox::new("unwritable-rust"),
        );
        for sandbox in [&c_box, &rust_box] {
            let fanout = sandbox.0.join("objects").join(&blocked[..2]);
            std::fs::write(fanout, b"not a directory").unwrap();
        }
        let mut c = CSide::open(&c_box.c_path());
        let rust_repo = Repository::open(&rust_box.c_path()).unwrap();
        for step in [
            Step::Annotation("blocked", COMMIT, TAGGER, message),
            Step::Annotated("blocked", COMMIT, TAGGER, message, false),
            Step::Annotated("blocked", COMMIT, TAGGER, message, true),
        ] {
            let c_seen = c.run(&step);
            assert!(
                matches!(c_seen, Outcome::Failed(..)),
                "{step:?}: {c_seen:?}"
            );
            assert_eq!(rust_run(&rust_repo, &other, &step), c_seen, "{step:?}");
            assert_eq!(tags(rust_repo.as_c_ptr()), tags(c.repo), "{step:?}");
        }
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

mod tag_accessors {
    //! Equivalence of the tag's getters and lookups ([`TagRef::id`], `owner`, `name`,
    //! `message`, `tagger`, `target_id`, `target_kind`, `target`, `peel`) with
    //! `git_tag_id`, `git_tag_owner`, `git_tag_name`, `git_tag_message`,
    //! `git_tag_tagger`, `git_tag_target_id`, `git_tag_target_type`, `git_tag_target`
    //! and `git_tag_peel`.
    //!
    //! Both sides open one copy of `testrepo.git` on their own, holding its five tags and
    //! tags written raw beside them: one whose target is missing, one whose recorded
    //! type is not its target's, one whose message holds a NUL, one without a message
    //! after its tagger, one with a malformed zone and one with an empty tagger name,
    //! a chain of tags ending in a blob, and one whose name holds spaces. Each side
    //! runs one script, in order: every tag is looked up, read, its target and peel
    //! taken and read, and then, with the tag released, the target and peel read again;
    //! a second pass reads every tag again from the cache, through a copy whose original
    //! is released first. After every step the sides compare every getter's value, as
    //! bytes, the owner (each side's own repository), and the target and peel (their id
    //! and type, or the code, error class and message).
    //!
    //! [`TagRef::id`]: libgit2::libgit2::object::tag::TagRef::id

    use std::ffi::{CStr, CString, c_int};
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::object::ObjectArc;
    use libgit2::libgit2::object::tag::{Tag, TagRef};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryError, RepositoryRef};

    use super::{c_last, init};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/c/tests/resources/testrepo.git"
    );
    /// Every tag of `testrepo.git`.
    const TAGS: [&str; 5] = [
        "b25fa35b38051e4ae45d4222e795f9df2e43f1d1",
        "7b4384978d2493e851f9cca7858815fac9b10980",
        "849a5e34a26815e821f865b8479f5815a47af0fe",
        "4a23e2e65ad4e31c4c9db7dc746650bfad082679",
        "521d87c1ec3aef9824daf6d96cc0ae3710766d91",
    ];
    const COMMIT: &str = "e90810b8df3e80c413d903f631643c716887138d";
    const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";
    const MISSING: &str = "dededededededededededededededededededede";

    /// Raw tag contents written beside `testrepo.git`'s tags; `{blob}` and `{tag}`
    /// name the chain's blob and its first tag.
    fn odd_tags() -> Vec<Vec<u8>> {
        let tagger = "tagger T <t@example.com> 1700000000 +0130\n";
        vec![
            format!("object {MISSING}\ntype commit\ntag missing\n{tagger}\nm\n").into_bytes(),
            format!("object {COMMIT}\ntype blob\ntag mistyped\n{tagger}\nm\n").into_bytes(),
            format!("object {COMMIT}\ntype commit\ntag nul\n{tagger}\nbefore\0after\n")
                .into_bytes(),
            format!("object {COMMIT}\ntype commit\ntag bare\n{tagger}").into_bytes(),
            format!("object {COMMIT}\ntype commit\ntag zone\ntagger Z <z@z> 5 +ab12\n\nz\n")
                .into_bytes(),
            format!("object {COMMIT}\ntype commit\ntag anon\ntagger  <> 7 -0700\n\n").into_bytes(),
            format!("object {COMMIT}\ntype commit\ntag na me\n{tagger}\nspaced\n").into_bytes(),
        ]
    }

    fn hex(id: *const ffi::git_oid) -> String {
        let mut out = [0u8; 65];
        // SAFETY: a live id, formatted into 65 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

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

    /// A private copy of `testrepo.git` holding [`odd_tags`] and a chain of tags of a
    /// blob, removed on drop; `ids` are every tag's, `testrepo.git`'s first.
    struct Sandbox {
        path: PathBuf,
        ids: Vec<String>,
    }

    impl Sandbox {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-tag-accessors-equiv-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            copy_dir(Path::new(TESTREPO), &path);
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let mut repo = ptr::null_mut();
            let mut odb = ptr::null_mut();
            // SAFETY: writable slots and a NUL-terminated path; each object is written
            // from a live buffer of its length; the odb and repository are released once.
            let ids = unsafe {
                assert_eq!(ffi::git_repository_open(&mut repo, c_path.as_ptr()), 0);
                assert_eq!(ffi::git_repository_odb(&mut odb, repo), 0);
                let write = |data: &[u8]| {
                    let mut id = ffi::git_oid::default();
                    let rc = ffi::git_odb_write(
                        &mut id,
                        odb,
                        data.as_ptr().cast(),
                        data.len(),
                        ffi::GIT_OBJECT_TAG,
                    );
                    assert_eq!(rc, 0);
                    hex(&id)
                };
                let mut ids: Vec<String> = TAGS.iter().map(|&t| t.to_owned()).collect();
                ids.extend(odd_tags().iter().map(|data| write(data)));
                let tagger = "tagger C <c@example.com> 1 +0000\n";
                let first = write(
                    format!("object {BLOB}\ntype blob\ntag chain0\n{tagger}\none\n").as_bytes(),
                );
                let second = write(
                    format!("object {first}\ntype tag\ntag chain1\n{tagger}\ntwo\n").as_bytes(),
                );
                ids.extend([first, second]);
                ffi::git_odb_free(odb);
                ffi::git_repository_free(repo);
                ids
            };
            Self { path, ids }
        }

        fn c_path(&self) -> CString {
            CString::new(self.path.to_str().unwrap()).unwrap()
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// An object a lookup gave, as its id and type, or the failure.
    type Looked = Result<(String, ffi::git_object_t), (c_int, c_int, String)>;

    /// The tagger: name and email (bytes, or `None` for NULL), seconds, offset, sign.
    type Tagger = (Option<Vec<u8>>, Option<Vec<u8>>, i64, i32, u8);

    /// What every getter of one tag reads.
    #[derive(Debug, PartialEq, Eq)]
    struct View {
        id: String,
        owned_by_its_repository: bool,
        name: Vec<u8>,
        message: Option<Vec<u8>>,
        tagger: Option<Tagger>,
        target_id: String,
        target_kind: ffi::git_object_t,
        target: Looked,
        peeled: Looked,
    }

    /// C's view of `object`, released after; or the failure `rc` left.
    fn c_looked(rc: c_int, object: *mut ffi::git_object) -> Looked {
        if rc < 0 {
            let (class, message) = c_last();
            return Err((rc, class, message));
        }
        // SAFETY: a live object the caller hands over, read, then released once.
        unsafe {
            let seen = (
                hex(ffi::git_object_id(object)),
                ffi::git_object_type(object),
            );
            ffi::git_object_free(object);
            Ok(seen)
        }
    }

    fn bytes(s: *const std::ffi::c_char) -> Option<Vec<u8>> {
        // SAFETY: NULL or a NUL-terminated string live for the copy.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) }.to_bytes().to_vec())
    }

    /// C's view of a live tag of `repo`.
    fn c_view(repo: *mut ffi::git_repository, tag: *mut ffi::git_tag) -> View {
        // SAFETY: a live tag of the live repository; every string and id read is the
        // tag's, copied at once; the target and peel are released by `c_looked`.
        unsafe {
            let tagger = ffi::git_tag_tagger(tag);
            let tagger = (!tagger.is_null()).then(|| {
                let t = &*tagger;
                (
                    bytes(t.name),
                    bytes(t.email),
                    t.when.time,
                    t.when.offset,
                    t.when.sign as u8,
                )
            });
            let mut target = ptr::null_mut();
            let rc = ffi::git_tag_target(&mut target, tag);
            let target = c_looked(rc, target);
            let mut peeled = ptr::null_mut();
            let rc = ffi::git_tag_peel(&mut peeled, tag);
            let peeled = c_looked(rc, peeled);
            View {
                id: hex(ffi::git_tag_id(tag)),
                owned_by_its_repository: ffi::git_tag_owner(tag) == repo,
                name: bytes(ffi::git_tag_name(tag)).unwrap(),
                message: bytes(ffi::git_tag_message(tag)),
                tagger,
                target_id: hex(ffi::git_tag_target_id(tag)),
                target_kind: ffi::git_tag_target_type(tag),
                target,
                peeled,
            }
        }
    }

    fn rust_looked(result: Result<ObjectArc<'_>, RepositoryError>) -> Looked {
        match result {
            Ok(object) => Ok((
                object.as_ref().id().to_hex_string().unwrap(),
                object.as_ref().kind().raw(),
            )),
            Err(e) => Err((
                e.raw_code(),
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            )),
        }
    }

    /// Rust's view of a tag; the owner is compared with `repo` by the address of its
    /// path, which each repository allocates for itself.
    fn rust_view(repo: RepositoryRef<'_>, tag: TagRef<'_, '_>) -> View {
        let tagger = tag.tagger().map(|t| {
            let when = t.when();
            (
                t.name().map(|s| s.to_bytes().to_vec()),
                t.email().map(|s| s.to_bytes().to_vec()),
                when.seconds(),
                when.offset_minutes(),
                when.sign(),
            )
        });
        View {
            id: tag.id().to_hex_string().unwrap(),
            owned_by_its_repository: tag.owner().path().map(CStr::as_ptr)
                == repo.path().map(CStr::as_ptr),
            name: tag.name().to_bytes().to_vec(),
            message: tag.message().map(|m| m.to_bytes().to_vec()),
            tagger,
            target_id: tag.target_id().to_hex_string().unwrap(),
            target_kind: tag.target_kind().raw(),
            target: rust_looked(tag.target()),
            peeled: rust_looked(tag.peel()),
        }
    }

    fn raw_id(hex: &str) -> ffi::git_oid {
        let mut id = ffi::git_oid::default();
        let hex = CString::new(hex).unwrap();
        // SAFETY: a writable id and a NUL-terminated hex string.
        let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        id
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_match_c() {
        init();
        let sandbox = Sandbox::new();
        let mut c_repo = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path.
        let rc = unsafe { ffi::git_repository_open(&mut c_repo, sandbox.c_path().as_ptr()) };
        assert_eq!(rc, 0);
        let rust_repo = Repository::open(&sandbox.c_path()).unwrap();
        let r = rust_repo.as_ref();
        let mut kinds = std::collections::HashSet::new();
        for pass in 0..2 {
            for (i, hex_id) in sandbox.ids.iter().enumerate() {
                let raw = raw_id(hex_id);
                let mut c_tag = ptr::null_mut();
                // SAFETY: a writable slot, the live repository and a live id of its type.
                assert_eq!(unsafe { ffi::git_tag_lookup(&mut c_tag, c_repo, &raw) }, 0);
                let id = Oid::from_string(&CString::new(hex_id.as_str()).unwrap(), OidType::Sha1)
                    .unwrap();
                let mut rust_tag = Tag::lookup(r, id.as_ref()).unwrap();
                if pass == 1 {
                    // Read through a copy whose original is released first.
                    let mut copy = ptr::null_mut();
                    // SAFETY: a live tag this side holds, copied, then released once.
                    unsafe {
                        assert_eq!(ffi::git_tag_dup(&mut copy, c_tag), 0);
                        ffi::git_tag_free(c_tag);
                    }
                    c_tag = copy;
                    let copy = rust_tag.as_ref().to_tag();
                    drop(std::mem::replace(&mut rust_tag, copy));
                }
                let c_seen = c_view(c_repo, c_tag);
                let rust_seen = rust_view(r, rust_tag.as_ref());
                assert_eq!(rust_seen, c_seen, "pass {pass}, tag {i}: {hex_id}");
                assert!(c_seen.owned_by_its_repository);
                kinds.insert((
                    c_seen.target.is_ok(),
                    c_seen.tagger.is_some(),
                    c_seen.message.is_some(),
                ));

                // The target and peel outlive the tag they came from.
                let mut c_target = ptr::null_mut();
                let mut c_peeled = ptr::null_mut();
                // SAFETY: writable slots and a live tag, released once after; a failure's
                // error is read at once, before the next call replaces it.
                let (c_target, c_peeled) = unsafe {
                    let rc = ffi::git_tag_target(&mut c_target, c_tag);
                    let target = (rc < 0).then(|| c_looked(rc, ptr::null_mut()));
                    let rc = ffi::git_tag_peel(&mut c_peeled, c_tag);
                    let peeled = (rc < 0).then(|| c_looked(rc, ptr::null_mut()));
                    ffi::git_tag_free(c_tag);
                    (
                        target.unwrap_or_else(|| c_looked(0, c_target)),
                        peeled.unwrap_or_else(|| c_looked(0, c_peeled)),
                    )
                };
                let rust_target = rust_tag.as_ref().target();
                let rust_peeled = rust_tag.as_ref().peel();
                drop(rust_tag);
                assert_eq!(
                    rust_looked(rust_target),
                    c_target,
                    "pass {pass}, target of tag {i}"
                );
                assert_eq!(
                    rust_looked(rust_peeled),
                    c_peeled,
                    "pass {pass}, peel of tag {i}"
                );
            }
        }
        // Found and missing targets, with and without a tagger and a message.
        assert!(kinds.len() >= 4, "{kinds:?}");
        // SAFETY: the raw repository is released once; nothing of it is held.
        unsafe { ffi::git_repository_free(c_repo) };
    }
}
