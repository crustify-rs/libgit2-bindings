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
//! contents through them: the code, the id, and the error class and message. Loading a
//! filter list initialises libgit2's filters unsynchronised (see `filter.rs`), so the
//! script runs alone in a child process.

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
    use libgit2::libgit2::object::object::ObjectIdOptions;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

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
