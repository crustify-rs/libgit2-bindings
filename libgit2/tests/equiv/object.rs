//! Equivalence of `git2/object.h`'s object owner and constructors with the C routines
//! they wrap.
//!
//! [`Object::lookup`] must find what `git_object_lookup` finds, and
//! [`Object::lookup_prefix`] what `git_object_lookup_prefix` does: the same object (id
//! and type, as C reads them) or the same failure (code, error class and message).
//! Each side opens `testrepo.git` on its own and runs one script of lookups, in order:
//! full ids and prefixes of 0 to past 40 digits, of every object type, under every type
//! filter, ambiguous prefixes, missing and zero ids, and repeats. After every step the
//! sides are compared, and so is which earlier result each new object is the same
//! object as (the cache hands back the object it holds). Then each side copies every
//! result (`git_object_dup`, `clone` and `ObjectRef::to_object`), releases the
//! originals, and compares the copies again, and the lookups after the releases. A
//! repository without a git directory is compared the same way, and so is one holding
//! objects libgit2 cannot parse (a commit, trees and a tag), beside a well-formed blob.
//!
//! Documented divergence, outside these comparisons: an id of another type than the
//! repository's is refused by the wrappers (code `-1`, `GIT_ERROR_INVALID`), where
//! the C routines format it out of bounds or read an uninitialised buffer; the unit
//! tests of `object.rs` hold the refusal.
//!
//! [`ObjectIdOptions`] must be what C code builds: [`ObjectIdOptions::new`] the struct
//! `git_object_id_options_init` fills (and `GIT_OBJECT_ID_OPTIONS_INIT` spells), and
//! each setter the field write C code makes. One script of steps runs on both sides,
//! each on its own options and its own filter lists, loaded from its own repository of
//! identical content: every object type and id type, `None` back to the defaults, a
//! `crlf` list, an empty list, and no list. After every step the sides compare the
//! options' fields as C reads them (the list as present or not, each side's pointer
//! being its own list's), and what `git_object_id_from_buffer` makes of each of a few
//! contents through them: the code, the id, and the error class and message; and what
//! [`id_from_buffer`] and [`id_from_file`] make of the same contents, as buffers and as
//! files (with a missing path and a directory), through Rust's options, against
//! `git_object_id_from_buffer` and `git_object_id_from_file` through C's. Loading a
//! filter list initialises libgit2's filters unsynchronised (see `filter.rs`), so the
//! script runs alone in a child process.
//!
//! [`id_from_buffer`]: libgit2::libgit2::object::object::id_from_buffer
//! [`id_from_file`]: libgit2::libgit2::object::object::id_from_file

use std::ffi::{CStr, CString, c_int};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::core::types::ObjectType;
use libgit2::libgit2::object::object::{Object, ObjectArc};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

const TESTREPO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../tests/resources/testrepo.git"
);

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

/// What a lookup gave: the object's id and type as C reads them, or the failure.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Found(String, ffi::git_object_t),
    Failed(c_int, c_int, String),
}

/// C's view of a live object.
fn found(p: *const ffi::git_object) -> Outcome {
    let mut out = [0u8; 65];
    // SAFETY: `p` is a live object; its id lives as long as it and is formatted into
    // `out`, 65 writable bytes, NUL included.
    unsafe {
        ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), ffi::git_object_id(p));
        Outcome::Found(
            CStr::from_bytes_until_nul(&out)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
            ffi::git_object_type(p),
        )
    }
}

/// One lookup of the script: an id (or prefix) in hex, a length (`None` for the full
/// lookup) and a type filter.
#[derive(Clone, Copy, Debug)]
struct Step<'a> {
    hex: &'a str,
    len: Option<usize>,
    kind: Option<ObjectType>,
}

/// A fresh id for `step`, owned by the side that uses it.
fn id_of(step: Step<'_>) -> Oid {
    Oid::from_prefix(step.hex.as_bytes(), OidType::Sha1).unwrap()
}

/// The C side: the raw repository and the references it looked up.
struct CSide {
    repo: *mut ffi::git_repository,
    held: Vec<*mut ffi::git_object>,
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

    /// The step's outcome, and the index of the earlier result it is the same object
    /// as; a found object is held.
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
        let kind = ObjectType::raw_or_any(step.kind);
        // SAFETY: a writable slot, the live repository, a live id of its type read for
        // the call, and a named type or `GIT_OBJECT_ANY`.
        let rc = unsafe {
            match step.len {
                None => ffi::git_object_lookup(&mut out, self.repo, &id, kind),
                Some(len) => ffi::git_object_lookup_prefix(&mut out, self.repo, &id, len, kind),
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

    /// Copies every held object (`git_object_dup`), releases the originals, and
    /// views the copies.
    fn copy_and_release(&mut self) -> Vec<Outcome> {
        let copies: Vec<_> = self
            .held
            .iter()
            .map(|&o| {
                let mut dest = ptr::null_mut();
                // SAFETY: `o` is a live reference this side holds.
                assert_eq!(unsafe { ffi::git_object_dup(&mut dest, o) }, 0);
                assert_eq!(dest, o);
                dest
            })
            .collect();
        for o in std::mem::replace(&mut self.held, copies) {
            // SAFETY: each original reference is released once; its copy remains.
            unsafe { ffi::git_object_free(o) };
        }
        self.held.iter().map(|&o| found(o)).collect()
    }
}

impl Drop for CSide {
    fn drop(&mut self) {
        // SAFETY: every held reference is released once, then the repository.
        unsafe {
            for &o in &self.held {
                ffi::git_object_free(o);
            }
            ffi::git_repository_free(self.repo);
        }
    }
}

/// C's view of what a wrapper gave.
fn outcome(result: &Result<ObjectArc<'_>, RepositoryError>) -> Outcome {
    match result {
        Ok(object) => found(object.as_c_ptr()),
        Err(e) => Outcome::Failed(
            e.raw_code(),
            e.raw_class(),
            e.message().to_string_lossy().into_owned(),
        ),
    }
}

/// The Rust side's step: the outcome, and the earlier result it is the same object as.
fn run_rust<'r>(
    repo: &'r RepositoryBox,
    held: &mut Vec<ObjectArc<'r>>,
    step: Step<'_>,
) -> (Outcome, Option<usize>) {
    let id = id_of(step);
    let result = match step.len {
        None => Object::lookup(repo.as_ref(), id.as_ref(), step.kind),
        Some(len) => Object::lookup_prefix(repo.as_ref(), id.as_ref(), len, step.kind),
    };
    let seen = outcome(&result);
    let Ok(object) = result else {
        return (seen, None);
    };
    let same = held.iter().position(|h| ObjectArc::ptr_eq(h, &object));
    held.push(object);
    (seen, same)
}

const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
const BLOB: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
const MISSING: &str = "dededededededededededededededededededede";
const ZERO: &str = "0000000000000000000000000000000000000000";

fn script() -> Vec<Step<'static>> {
    use ObjectType::*;
    let kinds = [None, Some(Commit), Some(Tree), Some(Blob), Some(Tag)];
    let mut steps = Vec::new();
    for hex in [COMMIT, TREE, BLOB, TAG, MISSING, ZERO] {
        for kind in kinds {
            steps.push(Step {
                hex,
                len: None,
                kind,
            });
        }
    }
    for len in [0, 1, 3, 4, 5, 7, 8, 20, 39, 40, 41, 64, usize::MAX] {
        for kind in [None, Some(Commit), Some(Tree)] {
            steps.push(Step {
                hex: COMMIT,
                len: Some(len),
                kind,
            });
        }
    }
    // Two trees start `1810`, a commit and a blob `3d35`; `1810d` is one of the trees.
    for hex in ["1810", "3d35", "1810d", "18103", "ffff", "a65fedf"] {
        for kind in kinds {
            steps.push(Step {
                hex,
                len: Some(hex.len()),
                kind,
            });
        }
    }
    steps.push(Step {
        hex: COMMIT,
        len: None,
        kind: None,
    });
    steps
}

/// Runs `script` on both sides, comparing after each step and after the copies.
fn compare(mut c: CSide, rust: &RepositoryBox, script: &[Step<'_>]) {
    let mut held = Vec::new();
    for &step in script {
        assert_eq!(c.run(step), run_rust(rust, &mut held, step), "{step:?}");
    }
    let c_copies = c.copy_and_release();
    let copies: Vec<_> = held
        .iter()
        .enumerate()
        .map(|(i, o)| {
            if i % 2 == 0 {
                o.clone()
            } else {
                o.as_ref().to_object()
            }
        })
        .collect();
    for (copy, original) in copies.iter().zip(&held) {
        assert!(ObjectArc::ptr_eq(copy, original));
    }
    drop(held);
    let rust_copies: Vec<_> = copies.iter().map(|o| found(o.as_c_ptr())).collect();
    assert_eq!(c_copies, rust_copies);

    // The same lookups again, with only the copies alive.
    let mut held = copies;
    for &step in script {
        assert_eq!(
            c.run(step),
            run_rust(rust, &mut held, step),
            "again: {step:?}"
        );
    }
}

#[test]
fn lookups_match_c() {
    init();
    let rust = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
    compare(CSide::open(TESTREPO), &rust, &script());
}

#[test]
fn lookups_in_a_repository_without_paths_match_c() {
    init();
    let rust = Repository::new().unwrap();
    let steps = [
        Step {
            hex: COMMIT,
            len: None,
            kind: None,
        },
        Step {
            hex: COMMIT,
            len: Some(7),
            kind: Some(ObjectType::Commit),
        },
        Step {
            hex: COMMIT,
            len: Some(3),
            kind: None,
        },
    ];
    compare(CSide::pathless(), &rust, &steps);
}

/// A repository holding objects libgit2 cannot parse, beside a well-formed one: each
/// lookup fails (or succeeds) alike, through every constructor and type filter.
#[test]
fn lookups_of_unparsable_objects_match_c() {
    init();
    let path = std::env::temp_dir().join(format!("crustify-object-equiv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let path_str = path.to_str().unwrap().to_owned();
    let objects: [(&[u8], ffi::git_object_t); 5] = [
        (b"tree nope\n", ffi::GIT_OBJECT_COMMIT),
        (b"100644 one\0short", ffi::GIT_OBJECT_TREE),
        (
            b"100644 a\0aaaaaaaaaaaaaaaaaaaa100644 b",
            ffi::GIT_OBJECT_TREE,
        ),
        (b"object 1234\n", ffi::GIT_OBJECT_TAG),
        (b"fine\n", ffi::GIT_OBJECT_BLOB),
    ];
    let ids: Vec<String> = {
        let repo = Repository::init(&CString::new(path_str.as_str()).unwrap(), true).unwrap();
        let odb = repo.as_ref().odb().unwrap();
        objects
            .iter()
            .map(|(data, kind)| {
                let mut id = ffi::git_oid::default();
                let mut out = [0u8; 41];
                // SAFETY: a live odb, a writable id, `data` read for the call; the id is
                // formatted into 41 writable bytes, NUL included.
                unsafe {
                    let rc = ffi::git_odb_write(
                        &mut id,
                        odb.as_c_ptr(),
                        data.as_ptr().cast(),
                        data.len(),
                        *kind,
                    );
                    assert_eq!(rc, 0);
                    ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), &id);
                }
                CStr::from_bytes_until_nul(&out)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    let mut steps = Vec::new();
    for hex in &ids {
        for kind in [
            None,
            Some(ObjectType::Commit),
            Some(ObjectType::Tree),
            Some(ObjectType::Blob),
            Some(ObjectType::Tag),
        ] {
            steps.push(Step {
                hex,
                len: None,
                kind,
            });
            steps.push(Step {
                hex,
                len: Some(12),
                kind,
            });
        }
    }
    let rust = Repository::open(&CString::new(path_str.as_str()).unwrap()).unwrap();
    compare(CSide::open(&path_str), &rust, &steps);
    drop(rust);
    let _ = std::fs::remove_dir_all(&path);
}

mod peel_and_bypath {
    //! Equivalence of [`ObjectRef::peel`] with `git_object_peel` and of
    //! [`ObjectRef::lookup_bypath`] with `git_object_lookup_bypath`.
    //!
    //! Each side opens `testrepo.git` on its own and looks up the same base objects: a
    //! commit, its tree, a blob, a tag of a tag of a commit, a tag of a blob, and the
    //! commit whose tree nests three levels of subtrees. From each base, every type
    //! filter is peeled to, the result peeled again to every filter (a chain through a
    //! nested tag), and every probe path looked up under every filter; after each step
    //! the sides compare the object found (id and type, as C reads it) or the failure
    //! (code, error class and message), and whether the result is the very object the
    //! step started from (`git_object_peel` of an object to its own type takes one more
    //! reference on it). Each side then releases the bases first, reads every derived
    //! object back, and releases those.
    //!
    //! [`ObjectRef::peel`]: libgit2::libgit2::object::object::ObjectRef::peel
    //! [`ObjectRef::lookup_bypath`]: libgit2::libgit2::object::object::ObjectRef::lookup_bypath

    use std::ffi::CStr;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;

    use super::{BLOB, COMMIT, CSide, Outcome, TESTREPO, TREE, c_last, found, init, outcome};

    /// `refs/tags/test`: a tag of tag `7b43849`, of commit `e90810b`.
    const NESTED_TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
    /// `refs/tags/annotated_tag_to_blob`: a tag of blob `1385f26`.
    const TAG_TO_BLOB: &str = "521d87c1ec3aef9824daf6d96cc0ae3710766d91";
    /// `refs/heads/subtrees`: its tree holds `ab/de/fgh/1.txt`.
    const SUBTREES: &str = "763d71aadf09a7951596c9746c024e7eece7c7af";
    const BASES: [&str; 6] = [COMMIT, TREE, BLOB, NESTED_TAG, TAG_TO_BLOB, SUBTREES];
    const KINDS: [Option<ObjectType>; 5] = [
        None,
        Some(ObjectType::Commit),
        Some(ObjectType::Tree),
        Some(ObjectType::Blob),
        Some(ObjectType::Tag),
    ];
    const PATHS: [&CStr; 16] = [
        c"README",
        c"ab",
        c"ab/",
        c"ab/de/",
        c"ab/c/3.txt",
        c"ab/de/fgh/1.txt",
        c"ab/de/fgh/1.txt/",
        c"",
        c"/",
        c"ab//de",
        c"README/",
        c"README/x",
        c"nope",
        c"ab/nope",
        c"./README",
        c"ab/../README",
    ];

    /// The C side's derived objects: each with whether it is its base.
    fn c_peel(
        base: *mut ffi::git_object,
        kind: Option<ObjectType>,
    ) -> (Outcome, bool, *mut ffi::git_object) {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot, a live object and a named type or `GIT_OBJECT_ANY`.
        let rc = unsafe { ffi::git_object_peel(&mut out, base, ObjectType::raw_or_any(kind)) };
        if rc < 0 {
            let (class, message) = c_last();
            return (Outcome::Failed(rc, class, message), false, ptr::null_mut());
        }
        (found(out), out == base, out)
    }

    fn c_bypath(
        base: *mut ffi::git_object,
        path: &CStr,
        kind: Option<ObjectType>,
    ) -> (Outcome, *mut ffi::git_object) {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot, a live object, a NUL-terminated path and a named type
        // or `GIT_OBJECT_ANY`.
        let rc = unsafe {
            ffi::git_object_lookup_bypath(
                &mut out,
                base,
                path.as_ptr(),
                ObjectType::raw_or_any(kind),
            )
        };
        if rc < 0 {
            let (class, message) = c_last();
            return (Outcome::Failed(rc, class, message), ptr::null_mut());
        }
        (found(out), out)
    }

    fn rust_peel<'r>(
        base: &ObjectArc<'r>,
        kind: Option<ObjectType>,
    ) -> (Outcome, bool, Option<ObjectArc<'r>>) {
        let result = base.as_ref().peel(kind);
        let seen = outcome(&result);
        match result {
            Ok(object) => {
                let same = ObjectArc::ptr_eq(&object, base);
                (seen, same, Some(object))
            }
            Err(_) => (seen, false, None),
        }
    }

    fn rust_bypath<'r>(
        base: &ObjectArc<'r>,
        path: &CStr,
        kind: Option<ObjectType>,
    ) -> (Outcome, Option<ObjectArc<'r>>) {
        let result = base.as_ref().lookup_bypath(path, kind);
        (outcome(&result), result.ok())
    }

    #[test]
    fn peels_and_path_lookups_match_c() {
        init();
        let c = CSide::open(TESTREPO);
        let rust = Repository::open(&std::ffi::CString::new(TESTREPO).unwrap()).unwrap();
        let mut c_held: Vec<*mut ffi::git_object> = Vec::new();
        let mut rust_held: Vec<ObjectArc<'_>> = Vec::new();
        let mut c_bases = Vec::new();
        let mut rust_bases = Vec::new();
        for hex in BASES {
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id and 40 readable hex digits.
            let rc = unsafe {
                ffi::git_oid_from_prefix(&mut id, hex.as_ptr().cast(), 40, ffi::GIT_OID_SHA1)
            };
            assert_eq!(rc, 0);
            let mut base = ptr::null_mut();
            // SAFETY: a writable slot, the live repository and a live id of its type.
            let rc = unsafe { ffi::git_object_lookup(&mut base, c.repo, &id, ffi::GIT_OBJECT_ANY) };
            assert_eq!(rc, 0);
            c_bases.push(base);
            let oid = Oid::from_prefix(hex.as_bytes(), OidType::Sha1).unwrap();
            rust_bases.push(Object::lookup(rust.as_ref(), oid.as_ref(), None).unwrap());
        }

        for (i, (&c_base, rust_base)) in c_bases.iter().zip(&rust_bases).enumerate() {
            for kind in KINDS {
                let (c_seen, c_same, c_out) = c_peel(c_base, kind);
                let (r_seen, r_same, r_out) = rust_peel(rust_base, kind);
                assert_eq!(
                    (c_seen, c_same),
                    (r_seen, r_same),
                    "peel {} to {kind:?}",
                    BASES[i]
                );
                // The peeled object, peeled once more to every type.
                if let (false, Some(r_out)) = (c_out.is_null(), r_out) {
                    for again in KINDS {
                        let (c_seen, c_same, c_again) = c_peel(c_out, again);
                        let (r_seen, r_same, r_again) = rust_peel(&r_out, again);
                        assert_eq!(
                            (c_seen, c_same),
                            (r_seen, r_same),
                            "peel {} to {kind:?} then {again:?}",
                            BASES[i]
                        );
                        if !c_again.is_null() {
                            c_held.push(c_again);
                            rust_held.push(r_again.unwrap());
                        }
                    }
                    c_held.push(c_out);
                    rust_held.push(r_out);
                }
            }
            for path in PATHS {
                for kind in KINDS {
                    let (c_seen, c_out) = c_bypath(c_base, path, kind);
                    let (r_seen, r_out) = rust_bypath(rust_base, path, kind);
                    assert_eq!(c_seen, r_seen, "{} {path:?} {kind:?}", BASES[i]);
                    if !c_out.is_null() {
                        c_held.push(c_out);
                        rust_held.push(r_out.unwrap());
                    }
                }
            }
        }
        assert_eq!(c_held.len(), rust_held.len());
        assert!(c_held.len() > 50, "{}", c_held.len());

        // The bases go first; every derived object is still readable, and read alike.
        for base in c_bases {
            // SAFETY: each base's reference is released once; the derived objects hold
            // their own.
            unsafe { ffi::git_object_free(base) };
        }
        drop(rust_bases);
        let c_views: Vec<_> = c_held.iter().map(|&o| found(o)).collect();
        let rust_views: Vec<_> = rust_held.iter().map(|o| found(o.as_c_ptr())).collect();
        assert_eq!(c_views, rust_views);
        for o in c_held {
            // SAFETY: each derived reference is released once.
            unsafe { ffi::git_object_free(o) };
        }
        drop(rust_held);
    }
}

/// `git_object_id_options`: [`ObjectIdOptions`] against the struct C code builds.
mod id_options {
    use super::{c_last, init};

    use std::ffi::{CStr, CString, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::filter::filter::{FilterList, FilterListBox};
    use libgit2::libgit2::object::object::{ObjectIdOptions, id_from_buffer, id_from_file};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process: libgit2 initialises a filter, unsynchronised,
    /// the first time any list selects it, and other modules' tests load lists through raw
    /// calls the crate cannot order (see `filter.rs`'s module docs).
    fn isolated(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// The fields of options as C reads them; the list as present or not.
    fn c_fields(
        opts: *const ffi::git_object_id_options,
    ) -> (u32, ffi::git_object_t, ffi::git_oid_t, bool) {
        // SAFETY: live options, read field by field.
        unsafe {
            (
                (*opts).version,
                (*opts).object_type,
                (*opts).oid_type,
                !(*opts).filters.is_null(),
            )
        }
    }

    /// What `git_object_id_from_buffer` makes of `data` through `opts`: the id's hex, or
    /// the code and the error.
    fn c_id(
        opts: *const ffi::git_object_id_options,
        data: &[u8],
    ) -> Result<String, (c_int, c_int, String)> {
        let mut out = ffi::git_oid::default();
        let bytes = if data.is_empty() { &[0u8][..] } else { data };
        // SAFETY: a writable id; `data.len()` readable bytes (a live one for none); live
        // options whose list, if any, is live and used by nothing else meanwhile.
        let rc = unsafe {
            ffi::git_object_id_from_buffer(&mut out, bytes.as_ptr().cast(), data.len(), opts)
        };
        if rc < 0 {
            let (class, message) = c_last();
            return Err((rc, class, message));
        }
        let mut hex = [0u8; 65];
        // SAFETY: the id libgit2 wrote; 65 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &out) };
        Ok(CStr::from_bytes_until_nul(&hex)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned())
    }

    /// What `git_object_id_from_file` makes of the file at `path` through `opts`, in the
    /// shape of [`c_id`].
    fn c_file_id(
        opts: *const ffi::git_object_id_options,
        path: &CStr,
    ) -> Result<String, (c_int, c_int, String)> {
        let mut out = ffi::git_oid::default();
        // SAFETY: a writable id; a NUL-terminated path; live options whose list, if any,
        // is live and used by nothing else meanwhile.
        let rc = unsafe { ffi::git_object_id_from_file(&mut out, path.as_ptr(), opts) };
        if rc < 0 {
            let (class, message) = c_last();
            return Err((rc, class, message));
        }
        let mut hex = [0u8; 65];
        // SAFETY: the id libgit2 wrote; 65 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &out) };
        Ok(CStr::from_bytes_until_nul(&hex)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned())
    }

    /// A wrapper's result in the shape of [`c_id`].
    fn outcome(result: Result<Oid, RepositoryError>) -> Result<String, (c_int, c_int, String)> {
        match result {
            Ok(id) => Ok(id.as_ref().to_hex_string().unwrap()),
            Err(e) => Err((
                e.raw_code(),
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            )),
        }
    }

    /// The contents as files of their own, `c<i>`, then a missing path and a
    /// directory; removed on drop.
    struct Files {
        dir: std::path::PathBuf,
        paths: Vec<CString>,
    }

    impl Files {
        fn new(side: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "crustify-id-files-equiv-{}-{side}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("directory")).unwrap();
            let mut paths = Vec::new();
            for (i, data) in CONTENTS.iter().enumerate() {
                let file = dir.join(format!("c{i}"));
                std::fs::write(&file, data).unwrap();
                paths.push(CString::new(file.to_str().unwrap()).unwrap());
            }
            for name in ["missing", "directory"] {
                paths.push(CString::new(dir.join(name).to_str().unwrap()).unwrap());
            }
            Self { dir, paths }
        }
    }

    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// C's ids of every content, from buffers then from files, through `opts`, with the
    /// failures' messages relative to each side's own directory.
    fn c_ids(
        opts: *const ffi::git_object_id_options,
        files: &Files,
    ) -> Vec<Result<String, (c_int, c_int, String)>> {
        let mut ids: Vec<_> = CONTENTS.iter().map(|d| c_id(opts, d)).collect();
        ids.extend(
            files
                .paths
                .iter()
                .map(|p| relative(c_file_id(opts, p), files)),
        );
        ids
    }

    /// The wrappers' ids, as [`c_ids`] lists C's.
    fn rust_ids(
        opts: &mut ObjectIdOptions<'_>,
        files: &Files,
    ) -> Vec<Result<String, (c_int, c_int, String)>> {
        let mut ids: Vec<_> = CONTENTS
            .iter()
            .map(|d| outcome(id_from_buffer(d, Some(opts.as_mut()))))
            .collect();
        ids.extend(
            files
                .paths
                .iter()
                .map(|p| relative(outcome(id_from_file(p, Some(opts.as_mut()))), files)),
        );
        ids
    }

    /// `result` with the side's directory dropped from a failure's message.
    fn relative(
        result: Result<String, (c_int, c_int, String)>,
        files: &Files,
    ) -> Result<String, (c_int, c_int, String)> {
        let dir = files.dir.to_str().unwrap();
        result.map_err(|(rc, class, message)| (rc, class, message.replace(dir, "<dir>")))
    }

    /// A repository with a working directory whose `.gitattributes` marks `*.txt` as text,
    /// so that the `crlf` filter turns CRLF into LF on the way into the database; removed
    /// on drop.
    struct Workdir {
        path: std::path::PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Workdir {
        /// `safecrlf` makes the repository's `crlf` filter refuse content whose CRLFs
        /// it would turn into LFs (`core.safecrlf`), so applying it fails.
        fn new(side: &str, safecrlf: bool) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-id-options-equiv-{}-{side}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let mut repo = Repository::init(&c_path, false).unwrap();
            std::fs::write(path.join(".gitattributes"), "*.txt text\n").unwrap();
            if safecrlf {
                drop(repo);
                let config = path.join(".git/config");
                let mut text = std::fs::read_to_string(&config).unwrap();
                text.push_str("[core]\n\tsafecrlf = true\n");
                std::fs::write(&config, text).unwrap();
                repo = Repository::open(&c_path).unwrap();
            }
            Self {
                path,
                repo: Some(repo),
            }
        }

        fn lists(&self) -> (FilterListBox<'_>, FilterListBox<'_>) {
            let repo = self.repo.as_ref().unwrap().as_ref();
            let crlf = FilterList::load(repo, c"a.txt", ffi::GIT_FILTER_TO_ODB, 0)
                .unwrap()
                .unwrap();
            let empty = FilterList::new(repo, ffi::GIT_FILTER_TO_ODB, 0).unwrap();
            (crlf, empty)
        }
    }

    impl Drop for Workdir {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// One step of the options script.
    #[derive(Clone, Copy, Debug)]
    enum IdStep {
        Object(Option<ObjectType>),
        Oid(Option<OidType>),
        Crlf,
        Empty,
        Safe,
        NoList,
    }

    /// Each list is lent once: the options keep a list borrowed for their whole life.
    const ID_SCRIPT: [IdStep; 23] = [
        IdStep::Object(Some(ObjectType::Commit)),
        IdStep::Object(Some(ObjectType::Tree)),
        IdStep::Oid(Some(OidType::Sha256)),
        IdStep::Object(Some(ObjectType::Tag)),
        IdStep::Object(None),
        IdStep::Oid(Some(OidType::Sha1)),
        IdStep::Crlf,
        IdStep::Oid(Some(OidType::Sha256)),
        IdStep::Object(Some(ObjectType::Blob)),
        IdStep::Oid(None),
        IdStep::Object(Some(ObjectType::Commit)),
        IdStep::Object(None),
        IdStep::Empty,
        IdStep::Object(Some(ObjectType::Blob)),
        IdStep::NoList,
        IdStep::Object(None),
        IdStep::Oid(Some(OidType::Sha256)),
        IdStep::Safe,
        IdStep::Oid(None),
        IdStep::Object(Some(ObjectType::Tag)),
        IdStep::Object(Some(ObjectType::Blob)),
        IdStep::NoList,
        IdStep::Object(None),
    ];

    const CONTENTS: [&[u8]; 4] = [b"", b"hello\n", b"a\r\nb\r\n", b"mixed\r\nends\n\r\n"];

    type Observed = (
        (u32, ffi::git_object_t, ffi::git_oid_t, bool),
        Vec<Result<String, (c_int, c_int, String)>>,
    );

    /// The options' fields, and C's ids of every content through them.
    fn observe(opts: *const ffi::git_object_id_options) -> Observed {
        (
            c_fields(opts),
            CONTENTS.iter().map(|d| c_id(opts, d)).collect(),
        )
    }

    /// Rust's options, read in place: `ObjectIdOptions` is transparent over the C struct.
    fn rust_raw(opts: &ObjectIdOptions<'_>) -> *const ffi::git_object_id_options {
        ptr::from_ref(opts).cast()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_init() {
        let mut c = ffi::git_object_id_options::default();
        // SAFETY: writable options, overwritten with the template.
        let rc =
            unsafe { ffi::git_object_id_options_init(&mut c, ffi::GIT_OBJECT_ID_OPTIONS_VERSION) };
        assert_eq!(rc, 0);
        let template = ffi::git_object_id_options {
            version: ffi::GIT_OBJECT_ID_OPTIONS_VERSION,
            ..Default::default()
        };
        let rust = ObjectIdOptions::new();
        let defaulted = ObjectIdOptions::default();
        assert_eq!(ObjectIdOptions::VERSION, c.version);
        assert_eq!(c_fields(rust_raw(&rust)), c_fields(&c));
        assert_eq!(c_fields(rust_raw(&defaulted)), c_fields(&c));
        assert_eq!(c_fields(&template), c_fields(&c));
        assert_eq!(rust.as_ref().version(), c.version);
        init();
        assert_eq!(observe(rust_raw(&rust)), observe(&c));
        // The wrappers, through the default options and through none, against C's.
        let (c_files, rust_files) = (Files::new("init-c"), Files::new("init-rust"));
        let mut rust = rust;
        let expected = c_ids(&c, &c_files);
        assert_eq!(rust_ids(&mut rust, &rust_files), expected);
        assert_eq!(c_ids(ptr::null(), &c_files), expected);
        let mut none: Vec<_> = CONTENTS
            .iter()
            .map(|d| outcome(id_from_buffer(d, None)))
            .collect();
        none.extend(
            rust_files
                .paths
                .iter()
                .map(|p| relative(outcome(id_from_file(p, None)), &rust_files)),
        );
        assert_eq!(none, expected);
        assert!(
            expected[CONTENTS.len()..]
                .iter()
                .filter(|r| r.is_err())
                .count()
                == 2
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn steps_match() {
        isolated(concat!(module_path!(), "::steps_match"), || {
            init();
            let (c_wd, rust_wd) = (Workdir::new("c", false), Workdir::new("rust", false));
            let (c_safe_wd, rust_safe_wd) = (
                Workdir::new("c-safe", true),
                Workdir::new("rust-safe", true),
            );
            let (c_safe, _) = c_safe_wd.lists();
            let (mut rust_safe, _) = rust_safe_wd.lists();
            let (c_crlf, c_empty) = c_wd.lists();
            let (mut rust_crlf, mut rust_empty) = rust_wd.lists();
            let mut c = ffi::git_object_id_options::default();
            // SAFETY: writable options, overwritten with the template.
            let rc = unsafe {
                ffi::git_object_id_options_init(&mut c, ffi::GIT_OBJECT_ID_OPTIONS_VERSION)
            };
            assert_eq!(rc, 0);
            let mut rust = ObjectIdOptions::new();
            let (c_files, rust_files) = (Files::new("steps-c"), Files::new("steps-rust"));
            assert_eq!(observe(&c), observe(rust_raw(&rust)), "initial");
            let (mut crlf, mut empty) = (Some(rust_crlf.as_mut()), Some(rust_empty.as_mut()));
            let mut safe = Some(rust_safe.as_mut());
            for (i, step) in ID_SCRIPT.into_iter().enumerate() {
                match step {
                    IdStep::Object(kind) => {
                        c.object_type = kind.map_or(0, ObjectType::raw);
                        rust.as_mut().set_object_type(kind);
                    }
                    IdStep::Oid(kind) => {
                        c.oid_type = kind.map_or(0, OidType::raw);
                        rust.as_mut().set_oid_type(kind);
                    }
                    IdStep::Crlf => {
                        c.filters = c_crlf.as_c_ptr();
                        rust.as_mut().set_filters(crlf.take());
                    }
                    IdStep::Empty => {
                        c.filters = c_empty.as_c_ptr();
                        rust.as_mut().set_filters(empty.take());
                    }
                    IdStep::Safe => {
                        c.filters = c_safe.as_c_ptr();
                        rust.as_mut().set_filters(safe.take());
                    }
                    IdStep::NoList => {
                        c.filters = ptr::null_mut();
                        rust.as_mut().set_filters(None);
                    }
                }
                let read = rust.as_ref();
                let expected_fields = (
                    read.version(),
                    read.object_type().map_or(0, ObjectType::raw),
                    read.oid_type().map_or(0, OidType::raw),
                    read.filters().is_some(),
                );
                assert_eq!(
                    c_fields(rust_raw(&rust)),
                    expected_fields,
                    "step {i}: {step:?}"
                );
                let seen = observe(&c);
                assert_eq!(seen, observe(rust_raw(&rust)), "step {i}: {step:?}");
                // The wrappers through Rust's options, against C through its own.
                let wrapped = rust_ids(&mut rust, &rust_files);
                assert_eq!(wrapped, c_ids(&c, &c_files), "step {i}: {step:?}");
                assert_eq!(wrapped[..CONTENTS.len()], seen.1[..], "step {i}: {step:?}");
                if let IdStep::Safe = step {
                    // The refusing list fails the CRLF content, on both sides alike.
                    let refused = (
                        -1,
                        ffi::GIT_ERROR_FILTER as c_int,
                        "CRLF would be replaced by LF in 'a.txt'".to_owned(),
                    );
                    assert_eq!(seen.1[2], Err(refused));
                    assert!(seen.1[1].is_ok());
                }
            }
            // The options are done with: the lists are each side's own again, freed
            // before their repositories.
            drop((rust_crlf, rust_empty, c_crlf, c_empty));
        });
    }
}

mod types_and_contents {
    //! Equivalence of [`ObjectType::name`] with `git_object_type2string`,
    //! [`ObjectType::from_name`] with `git_object_string2type`, [`type_is_valid`] with
    //! `git_object_type_is_valid`, and [`content_is_valid`] with
    //! `git_object_rawcontent_is_valid`.
    //!
    //! The names are compared over every `git_object_t` value around the named ones (a
    //! value without a variant is the empty name in C, and no [`ObjectType`] in Rust),
    //! and over a corpus of names: the four, their prefixes and extensions, case
    //! changes, and the empty one. The content check runs C and Rust on independent
    //! copies of each content, for every object type and both id types, and compares
    //! the return code, the verdict and the calling thread's last error after each
    //! call, without clearing it in between: a commit, a tree and a tag of each id type,
    //! every prefix of each, and single-byte corruptions.
    //!
    //! Outside these comparisons: C also takes a NULL buffer (refused), an out-of-range
    //! type (refused as not found) and an unnamed id type; the wrapper's arguments
    //! cannot carry them. Out-of-memory failures are compared in `object.rs`'s unit
    //! tests.
    //!
    //! [`ObjectType::name`]: libgit2::libgit2::core::types::ObjectType::name
    //! [`ObjectType::from_name`]: libgit2::libgit2::core::types::ObjectType::from_name

    use std::ffi::{CStr, CString, c_int};

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::object::{content_is_valid, type_is_valid};
    use libgit2::libgit2::oid::oid::OidType;

    use super::{c_last, init};

    const KINDS: [ObjectType; 4] = [
        ObjectType::Commit,
        ObjectType::Tree,
        ObjectType::Blob,
        ObjectType::Tag,
    ];

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn names_match_c() {
        init();
        for raw in (-5..12).chain([i32::MIN, i32::MAX]) {
            // SAFETY: a plain value; the result is a static string.
            let c_name = unsafe { CStr::from_ptr(ffi::git_object_type2string(raw)) };
            let rust_name = ObjectType::from_raw(raw).map_or("", ObjectType::name);
            assert_eq!(c_name.to_str().unwrap(), rust_name, "{raw}");
            // SAFETY: a plain value.
            let c_valid = unsafe { ffi::git_object_type_is_valid(raw) } != 0;
            assert_eq!(type_is_valid(raw), c_valid, "{raw}");
        }
        let mut names: Vec<String> = ["", " ", "any", "invalid", "ofs_delta", "ref_delta"]
            .map(String::from)
            .to_vec();
        for name in ["commit", "tree", "blob", "tag"] {
            for len in 0..=name.len() {
                names.push(name[..len].to_owned());
            }
            names.push(format!("{name}s"));
            names.push(format!("{name}\t"));
            names.push(format!(" {name}"));
            names.push(name.to_uppercase());
            names.push(format!("{}{}", name[..1].to_uppercase(), &name[1..]));
        }
        names.push("\u{e9}".into());
        for name in names {
            let c_copy = CString::new(name.clone()).unwrap();
            let rust_copy = CString::new(name.clone()).unwrap();
            // SAFETY: a NUL-terminated string, read for the call.
            let c = unsafe { ffi::git_object_string2type(c_copy.as_ptr()) };
            let rust =
                ObjectType::from_name(&rust_copy).map_or(ffi::GIT_OBJECT_INVALID, ObjectType::raw);
            assert_eq!(rust, c, "{name:?}");
        }
    }

    /// A commit, a tree and a tag whose ids are of `kind`.
    fn samples(kind: OidType) -> Vec<Vec<u8>> {
        let (hex, raw) = match kind {
            OidType::Sha1 => ("a65fedf39aefe402d3bb6e24df4d4f5fe4547750", vec![0xa6; 20]),
            _ => (
                "f2a108f86a3b4fd9ad75ed55e9cb3cb46e348fca3b9dba3db64f7c9f64b8a736",
                vec![0xf2; 32],
            ),
        };
        let commit = format!(
            "tree {hex}\nparent {hex}\nauthor A U Thor <a@example.com> 1 +0100\n\
             committer C <c@example.com> 2 -0030\nencoding UTF-8\n\nsubject\n\nbody\n"
        );
        let tag =
            format!("object {hex}\ntype commit\ntag v1.0\ntagger T <t@example.com> 3 +0000\n\nm\n");
        let mut tree = Vec::new();
        for (mode, name) in [("100644", "README"), ("40000", "ab"), ("160000", "sub")] {
            tree.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            tree.extend_from_slice(&raw);
        }
        vec![commit.into_bytes(), tree, tag.into_bytes()]
    }

    /// The corpus: every sample of either id type, every prefix of each, and each with
    /// one byte flipped at a few places.
    fn corpus() -> Vec<Vec<u8>> {
        let mut corpus = vec![Vec::new(), b"\n".to_vec(), b"\0".to_vec()];
        for kind in [OidType::Sha1, OidType::Sha256] {
            for sample in samples(kind) {
                for len in 0..sample.len() {
                    corpus.push(sample[..len].to_vec());
                }
                for at in [0, 5, sample.len() / 2, sample.len() - 1] {
                    for byte in *b"\n\0 x" {
                        let mut flipped = sample.clone();
                        flipped[at] = byte;
                        corpus.push(flipped);
                    }
                }
                corpus.push(sample);
            }
        }
        corpus
    }

    /// What a check gave: the verdict or the code, and the last error after it.
    type Seen = (Result<bool, c_int>, (c_int, String));

    fn c_check(content: &[u8], kind: ObjectType, oid_type: OidType) -> Seen {
        let copy = content.to_vec();
        // C refuses a NULL buffer; lend a byte for no content, as the wrapper does.
        let buf = if copy.is_empty() {
            c"".as_ptr()
        } else {
            copy.as_ptr().cast()
        };
        let mut valid: c_int = -1;
        // SAFETY: `valid` is writable; `buf` addresses `copy.len()` readable bytes (one
        // for none), read for the call; named types.
        let rc = unsafe {
            ffi::git_object_rawcontent_is_valid(
                &mut valid,
                buf,
                copy.len(),
                kind.raw(),
                oid_type.raw(),
            )
        };
        let verdict = if rc < 0 {
            Err(rc)
        } else {
            assert!(valid == 0 || valid == 1, "C wrote valid = {valid}");
            Ok(valid == 1)
        };
        (verdict, c_last())
    }

    fn rust_check(content: &[u8], kind: ObjectType, oid_type: OidType) -> Seen {
        let copy = content.to_vec();
        let verdict = content_is_valid(&copy, kind, oid_type).map_err(|e| e.raw_class());
        (verdict, c_last())
    }

    /// The whole corpus, for every type and id type, in one sequence per side without
    /// clearing the error, compared step by step.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn content_checks_match_c() {
        init();
        let corpus = corpus();
        let mut probes = Vec::new();
        for content in &corpus {
            for kind in KINDS {
                for oid_type in [OidType::Sha1, OidType::Sha256] {
                    probes.push((content, kind, oid_type));
                }
            }
        }
        // SAFETY: no arguments; libgit2 is initialised.
        unsafe { ffi::git_error_clear() };
        let c: Vec<Seen> = probes.iter().map(|&(b, k, o)| c_check(b, k, o)).collect();
        // SAFETY: as above.
        unsafe { ffi::git_error_clear() };
        let rust: Vec<Seen> = probes
            .iter()
            .map(|&(b, k, o)| rust_check(b, k, o))
            .collect();
        let mut valid = 0;
        for ((probe, c), rust) in probes.iter().zip(&c).zip(&rust) {
            assert_eq!(
                rust,
                c,
                "{:?} as {:?} with {:?} ids",
                String::from_utf8_lossy(probe.0),
                probe.1,
                probe.2
            );
            valid += usize::from(c.0 == Ok(true));
        }
        // Both verdicts occur for every type but blobs.
        assert!(valid > corpus.len() * 2 && valid < probes.len(), "{valid}");
        for kind in [OidType::Sha1, OidType::Sha256] {
            for (sample, object) in
                samples(kind)
                    .iter()
                    .zip([ObjectType::Commit, ObjectType::Tree, ObjectType::Tag])
            {
                assert_eq!(rust_check(sample, object, kind).0, Ok(true), "{object:?}");
            }
        }
    }
}

mod kind_and_short_id {
    //! Equivalence of [`ObjectRef::kind`] with `git_object_type` and of
    //! [`ObjectRef::short_id`] with `git_object_short_id`.
    //!
    //! Each `core.abbrev` setting gets a copy of `testrepo.git` whose configuration sets
    //! it (none, 4, 5, 40, 41, `no`, `auto`, and the refused 3, `true` and `x`). Each side opens
    //! the copy on its own, looks up the same objects (one of each type, and pairs whose
    //! ids share 4 or 5 digits, so the abbreviation must grow), and compares, for each in
    //! turn, the type, and the short id or the failure (code, error class and message).
    //! Each side then asks again after releasing every other object, and compares again.
    //!
    //! [`ObjectRef::kind`]: libgit2::libgit2::object::object::ObjectRef::kind
    //! [`ObjectRef::short_id`]: libgit2::libgit2::object::object::ObjectRef::short_id

    use std::ffi::{CStr, CString, c_int};
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;

    use super::{BLOB, COMMIT, CSide, TAG, TESTREPO, TREE, c_last, init};

    /// Objects to abbreviate: one of each type, then pairs sharing 5 and 4 digits.
    const IDS: [&str; 10] = [
        COMMIT,
        TREE,
        BLOB,
        TAG,
        "498bc0906810bd43c6fbc73385fecb7f2d04be3a",
        "498bccdfec1fc223c27c0d84030ff419058e452d",
        "4260699b37a7a0cc6be231d232d424a858a7d111",
        "42608eb49e4be32002462b51e86065368b342c05",
        "763d71aadf09a7951596c9746c024e7eece7c7af",
        "763dd65d1e7ca14658649d7c4d1cc1e4bfd2c671",
    ];

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

    /// A copy of `testrepo.git` whose configuration sets `core.abbrev` to `abbrev`,
    /// removed on drop.
    struct Copy(PathBuf);

    impl Copy {
        fn new(abbrev: Option<&str>) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-object-equiv-{}-abbrev-{}",
                std::process::id(),
                abbrev.unwrap_or("default")
            ));
            let _ = std::fs::remove_dir_all(&path);
            copy_dir(Path::new(TESTREPO), &path);
            if let Some(abbrev) = abbrev {
                let config = path.join("config");
                let mut text = std::fs::read_to_string(&config).unwrap();
                text.push_str(&format!("[core]\n\tabbrev = {abbrev}\n"));
                std::fs::write(&config, text).unwrap();
            }
            Self(path)
        }
    }

    impl Drop for Copy {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// What one object gave: its type, and its short id or the failure.
    type Seen = (ffi::git_object_t, Result<String, (c_int, c_int, String)>);

    fn c_seen(object: *mut ffi::git_object) -> Seen {
        let mut buf = ffi::git_buf {
            ptr: ptr::null_mut(),
            reserved: 0,
            size: 0,
        };
        // SAFETY: a live object of a live repository; `buf` an empty buffer, disposed
        // once read.
        unsafe {
            let kind = ffi::git_object_type(object);
            let rc = ffi::git_object_short_id(&mut buf, object);
            let short = if rc < 0 {
                let (class, message) = c_last();
                Err((rc, class, message))
            } else {
                let short = CStr::from_ptr(buf.ptr).to_str().unwrap().to_owned();
                assert_eq!(short.len(), buf.size);
                Ok(short)
            };
            if buf.reserved > 0 {
                ffi::git_buf_dispose(&mut buf);
            }
            (kind, short)
        }
    }

    fn rust_seen(object: &ObjectArc<'_>) -> Seen {
        let short = match object.as_ref().short_id() {
            Ok(buf) => Ok(buf.as_ref().to_str().unwrap().to_owned()),
            Err(e) => Err((
                e.raw_code(),
                e.raw_class(),
                e.message().to_string_lossy().into_owned(),
            )),
        };
        (object.as_ref().kind().raw(), short)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn kinds_and_short_ids_match_c() {
        init();
        let settings = [
            None,
            Some("4"),
            Some("5"),
            Some("40"),
            Some("41"),
            Some("no"),
            Some("auto"),
            Some("true"),
            Some("3"),
            Some("x"),
        ];
        for abbrev in settings {
            let copy = Copy::new(abbrev);
            let path = copy.0.to_str().unwrap();
            let mut c = CSide::open(path);
            let rust_repo = Repository::open(&CString::new(path).unwrap()).unwrap();
            let mut rust = Vec::new();
            let mut c_first = Vec::new();
            let mut rust_first = Vec::new();
            for hex in IDS {
                let mut id = ffi::git_oid::default();
                let c_hex = CString::new(hex).unwrap();
                let mut out = ptr::null_mut();
                // SAFETY: writable slots, the side's live repository and a live id.
                unsafe {
                    assert_eq!(
                        ffi::git_oid_from_string(&mut id, c_hex.as_ptr(), ffi::GIT_OID_SHA1),
                        0
                    );
                    assert_eq!(
                        ffi::git_object_lookup(&mut out, c.repo, &id, ffi::GIT_OBJECT_ANY),
                        0
                    );
                }
                c.held.push(out);
                let rust_id = Oid::from_string(&c_hex, OidType::Sha1).unwrap();
                rust.push(Object::lookup(rust_repo.as_ref(), rust_id.as_ref(), None).unwrap());
                let (cs, rs) = (c_seen(out), rust_seen(rust.last().unwrap()));
                assert_eq!(rs, cs, "{abbrev:?} {hex}");
                c_first.push(cs);
                rust_first.push(rs);
            }
            // Every other object goes; the first one is asked again.
            for o in c.held.drain(1..) {
                // SAFETY: each reference is released once.
                unsafe { ffi::git_object_free(o) };
            }
            rust.truncate(1);
            let (cs, rs) = (c_seen(c.held[0]), rust_seen(&rust[0]));
            assert_eq!(rs, cs, "{abbrev:?} again");
            assert_eq!(cs, c_first[0]);
            match abbrev {
                Some("3") | Some("x") | Some("true") => {
                    assert!(c_first.iter().all(|s| s.1.is_err()))
                }
                Some("4") => assert_eq!(c_first[4].1.as_deref(), Ok("498bc0")),
                _ => assert!(
                    c_first.iter().all(|s| s.1.is_ok()),
                    "{abbrev:?}: {c_first:?}"
                ),
            }
        }
    }
}

/// Equivalence of `ObjectRef::id` and `ObjectRef::owner` with `git_object_id` and
/// `git_object_owner`.
///
/// Each side opens its own `testrepo.git` and looks up every kind of object by full id
/// and by prefix, and derives objects by peeling and by path. For each object both read
/// its id (as hex and type byte), and the repository it reports, which on each side must
/// be the very repository it was looked up in (same address for C; the same path string
/// for Rust, which the handle lends). The ids are compared, before and after the other
/// owners of the object are released.
mod id_and_owner {
    use std::ffi::{CStr, CString};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;

    use super::{BLOB, COMMIT, TAG, TESTREPO, TREE, init};

    /// One object, by full id or by its first 7 digits, or derived from one.
    #[derive(Clone, Copy, Debug)]
    enum Find {
        Full(&'static str),
        Prefix(&'static str),
        Peel(&'static str, ffi::git_object_t),
        Path(&'static str, &'static CStr),
    }

    const FINDS: [Find; 9] = [
        Find::Full(COMMIT),
        Find::Full(TREE),
        Find::Full(BLOB),
        Find::Full(TAG),
        Find::Prefix(COMMIT),
        Find::Prefix(TAG),
        Find::Peel(TAG, ffi::GIT_OBJECT_TREE),
        Find::Peel(COMMIT, ffi::GIT_OBJECT_TREE),
        Find::Path(COMMIT, c"README"),
    ];

    fn sha1(hex: &str) -> ffi::git_oid {
        let mut id = ffi::git_oid::default();
        let hex = CString::new(hex).unwrap();
        // SAFETY: a writable id and a NUL-terminated hex string.
        let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        id
    }

    fn c_find(repo: *mut ffi::git_repository, find: Find) -> *mut ffi::git_object {
        let mut out = ptr::null_mut();
        // SAFETY: writable slots, the live repository, live ids and paths; every
        // intermediate object is freed once.
        unsafe {
            let rc = match find {
                Find::Full(hex) => {
                    ffi::git_object_lookup(&mut out, repo, &sha1(hex), ffi::GIT_OBJECT_ANY)
                }
                Find::Prefix(hex) => ffi::git_object_lookup_prefix(
                    &mut out,
                    repo,
                    &sha1(hex),
                    7,
                    ffi::GIT_OBJECT_ANY,
                ),
                Find::Peel(hex, _) | Find::Path(hex, _) => {
                    let mut from = ptr::null_mut();
                    assert_eq!(
                        ffi::git_object_lookup(&mut from, repo, &sha1(hex), ffi::GIT_OBJECT_ANY),
                        0
                    );
                    let rc = match find {
                        Find::Peel(_, kind) => ffi::git_object_peel(&mut out, from, kind),
                        Find::Path(_, path) => ffi::git_object_lookup_bypath(
                            &mut out,
                            from,
                            path.as_ptr(),
                            ffi::GIT_OBJECT_ANY,
                        ),
                        _ => unreachable!(),
                    };
                    ffi::git_object_free(from);
                    rc
                }
            };
            assert_eq!(rc, 0, "{find:?}");
        }
        out
    }

    fn c_id(object: *const ffi::git_object) -> (String, u8) {
        let mut out = [0u8; 65];
        // SAFETY: a live object, whose id lives as long as it; 65 writable bytes.
        unsafe {
            let id = ffi::git_object_id(object);
            ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id);
            (
                CStr::from_bytes_until_nul(&out)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
                (*id).type_ as u8,
            )
        }
    }

    fn rust_find<'r>(
        repo: libgit2::libgit2::repository::repository::RepositoryRef<'r>,
        find: Find,
    ) -> ObjectArc<'r> {
        let id = |hex: &str| Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap();
        match find {
            Find::Full(hex) => Object::lookup(repo, id(hex).as_ref(), None),
            Find::Prefix(hex) => Object::lookup_prefix(repo, id(hex).as_ref(), 7, None),
            Find::Peel(hex, kind) => Object::lookup(repo, id(hex).as_ref(), None)
                .unwrap()
                .as_ref()
                .peel(ObjectType::from_raw(kind)),
            Find::Path(hex, path) => Object::lookup(repo, id(hex).as_ref(), None)
                .unwrap()
                .as_ref()
                .lookup_bypath(path, None),
        }
        .unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn ids_and_owners_match_c() {
        init();
        let path = CString::new(TESTREPO).unwrap();
        let mut c_repo = ptr::null_mut();
        // SAFETY: a writable slot and a NUL-terminated path.
        let rc = unsafe { ffi::git_repository_open(&mut c_repo, path.as_ptr()) };
        assert_eq!(rc, 0);
        let rust_repo = Repository::open(&path).unwrap();
        let repo = rust_repo.as_ref();
        for find in FINDS {
            let c_object = c_find(c_repo, find);
            let object = rust_find(repo, find);
            let copy = object.clone();
            let rust_id = object.as_ref().id();
            let rust = (rust_id.to_hex_string().unwrap(), rust_id.raw_kind());
            assert_eq!(rust, c_id(c_object), "{find:?}");
            // SAFETY: a live object, only read.
            assert_eq!(unsafe { ffi::git_object_owner(c_object) }, c_repo);
            let owner = object.as_ref().owner();
            assert_eq!(
                owner.path().map(CStr::as_ptr),
                repo.path().map(CStr::as_ptr)
            );
            drop(object);
            assert_eq!(copy.as_ref().id().to_hex_string().unwrap(), rust.0);
            // SAFETY: the object C looked up, freed once.
            unsafe { ffi::git_object_free(c_object) };
        }
        // SAFETY: the C side's repository, freed once.
        unsafe { ffi::git_repository_free(c_repo) };
    }
}
