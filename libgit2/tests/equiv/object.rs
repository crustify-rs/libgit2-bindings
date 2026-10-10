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
