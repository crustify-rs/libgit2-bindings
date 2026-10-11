//! Equivalence of `git2/commit.h`'s [`CommitArray`] with the `git_commitarray` C code
//! holds, has libgit2 fill and releases with `git_commitarray_dispose`.
//!
//! Each array is produced twice, independently, one copy per side, by
//! `git_repository_commit_parents` on its own open repository: `testrepo.git` (one
//! parent), `mergedrepo` (a merge in progress: `HEAD` and `MERGE_HEAD`) and
//! `empty_bare.git` (an unborn `HEAD`: `{NULL, 0}`). The Rust side adopts its copy as
//! libgit2 hands it over. Both sides then read the array (count, and each commit's
//! id), release it (C: `git_commitarray_dispose`; Rust: [`CommitArrayMut::clear`]),
//! have libgit2 fill it again, read it, and release it for good (C: dispose; Rust:
//! drop) before the repository, which the Rust array borrows. After every step both
//! record the struct's fields as the public API exposes them and the results are
//! compared.
//!
//! Documented divergences, outside these comparisons:
//!
//! - An empty array (`commits` NULL) never reaches libgit2 in Rust:
//!   `CommitArrayDispose` skips the C call, which would be `git__free(NULL)` reading
//!   the allocator unsynchronised against a concurrent first `git_libgit2_init`. Both
//!   sides end in `{NULL, 0}` either way.
//! - A handle over an array whose `commits` is NULL with a non-zero `count` (no
//!   libgit2 path produces one) reads as empty and clears to `{NULL, 0}` in Rust; C's
//!   dispose would dereference NULL. The module's `unit_tests` cover it.

use std::ffi::{CStr, c_int};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef, CommitArrayVal, CommitRef};

const TESTREPO: &CStr = c"tests/resources/testrepo.git";
const MERGEDREPO: &CStr = c"tests/resources/mergedrepo/.gitted";
const EMPTY: &CStr = c"tests/resources/empty_bare.git";

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

/// Opens the fixture repository `name`, relative to the libgit2 checkout.
fn open(name: &CStr) -> *mut ffi::git_repository {
    init();
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../");
    let path = std::ffi::CString::new(format!("{root}{}", name.to_str().unwrap())).unwrap();
    let mut repo = ptr::null_mut();
    // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no ceiling
    // directories.
    let rc = unsafe {
        ffi::git_repository_open_ext(
            &mut repo,
            path.as_ptr(),
            ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
            ptr::null(),
        )
    };
    assert_eq!(rc, 0);
    repo
}

/// The hex id of the live commit `commit`.
fn hex(commit: *mut ffi::git_commit) -> String {
    let mut out = [0u8; 41];
    // SAFETY: `commit` is a live commit its array keeps alive; its id is formatted
    // into `out`, 41 writable bytes, NUL included.
    unsafe {
        ffi::git_oid_tostr(
            out.as_mut_ptr().cast(),
            out.len(),
            ffi::git_commit_id(commit),
        )
    };
    CStr::from_bytes_until_nul(&out)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// What one side observes of an array: `(count, commit ids)`.
type Record = (usize, Vec<String>);

/// The C side's view of `raw`: `count` and the id of each of its commits.
fn c_record(raw: &ffi::git_commitarray) -> Record {
    let ids = (0..raw.count)
        // SAFETY: libgit2 left `commits` holding `count` live commit pointers.
        .map(|i| hex(unsafe { *raw.commits.add(i) }))
        .collect();
    (raw.count, ids)
}

/// The hex id of a commit the Rust side lends; the id is read through a copied owner's
/// pointer, standing in for the `git_commit_id` wrapper.
fn rust_hex(commit: CommitRef<'_, '_>) -> String {
    hex(commit.to_commit().as_c_ptr())
}

/// The Rust side's view, through the public API only.
fn rust_record(array: CommitArrayRef<'_, '_>) -> Record {
    (array.len(), array.iter().map(rust_hex).collect())
}

/// `git_repository_commit_parents` into `raw`, which must hold nothing.
fn c_fill(raw: &mut ffi::git_commitarray, repo: *mut ffi::git_repository) -> i32 {
    // SAFETY: `raw` is writable and empty, so nothing is overwritten unreleased;
    // `repo` is open.
    unsafe { ffi::git_repository_commit_parents(raw, repo) }
}

/// The same fill, adopted by the Rust side, which releases it before `repo`.
fn rust_fill<'r>(repo: *mut ffi::git_repository) -> (i32, CommitArrayVal<'r>) {
    let mut raw = ffi::git_commitarray::default();
    let rc = c_fill(&mut raw, repo);
    // SAFETY: `raw` is what libgit2 left: `{NULL, 0}`, or a buffer of references to the
    // fixtures' commits (their `HEAD`s and `MERGE_HEAD`s are commits) from its
    // allocator, given up here; the caller releases it before freeing `repo`.
    (rc, unsafe { CommitArray::from_raw(raw) })
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_arrays_match() {
    // C's dispose of the empty array still reaches `git__free(NULL)`, which reads the
    // allocator a concurrent first `git_libgit2_init` writes: initialise first.
    init();
    let mut raw = ffi::git_commitarray::default();
    let mut array: CommitArrayVal<'_> = CommitArray::new();
    assert_eq!(c_record(&raw), rust_record(array.as_ref()));
    // SAFETY: `raw` holds nothing; the dispose accepts the empty array.
    unsafe { ffi::git_commitarray_dispose(&mut raw) };
    array.as_mut().clear();
    assert_eq!(c_record(&raw), rust_record(array.as_ref()));
    let back = array.into_inner().into_raw();
    assert_eq!((raw.commits, raw.count), (back.commits, back.count));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn filled_arrays_match_through_their_lifecycle() {
    for (name, expected) in [(TESTREPO, 1), (MERGEDREPO, 2), (EMPTY, 0)] {
        let (c_repo, rust_repo) = (open(name), open(name));

        let mut raw = ffi::git_commitarray::default();
        let c_rc = c_fill(&mut raw, c_repo);
        let (rust_rc, mut array) = rust_fill(rust_repo);
        assert_eq!(c_rc, rust_rc, "{name:?}: fill");
        let filled = c_record(&raw);
        assert_eq!(filled.0, expected, "{name:?}");
        assert_eq!(filled, rust_record(array.as_ref()), "{name:?}: fill");
        assert_eq!(raw.commits.is_null(), array.as_ref().get(0).is_none());

        // SAFETY: `raw` owns what libgit2 filled it with, released once.
        unsafe { ffi::git_commitarray_dispose(&mut raw) };
        array.as_mut().clear();
        assert_eq!(
            c_record(&raw),
            rust_record(array.as_ref()),
            "{name:?}: release"
        );
        assert!(raw.commits.is_null());

        // Filled again: the C side reuses its struct; the Rust side hands its cleared
        // value back to C, as the wrapper's out-parameter does.
        let c_rc = c_fill(&mut raw, c_repo);
        let mut cleared = array.into_inner().into_raw();
        let rust_rc = c_fill(&mut cleared, rust_repo);
        // SAFETY: as in `rust_fill`.
        let array: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(cleared) };
        assert_eq!(c_rc, rust_rc, "{name:?}: refill");
        assert_eq!(c_record(&raw), filled);
        assert_eq!(rust_record(array.as_ref()), filled, "{name:?}: refill");

        // SAFETY: `raw` owns its refill, released once.
        unsafe { ffi::git_commitarray_dispose(&mut raw) };
        drop(array);
        assert_eq!(c_record(&raw), (0, Vec::new()));
        for repo in [c_repo, rust_repo] {
            // SAFETY: each repository is open and freed once, after its array.
            unsafe { ffi::git_repository_free(repo) };
        }
    }
}

/// Equivalence of [`Header`] with the `git_commit_header` arrays C code builds for
/// `git_commit_create_ext_options.extra_headers`.
///
/// Each side owns a fresh bare repository and builds the same chain of commits in it,
/// each commit's parent the side's previous one, over the empty tree, with fixed
/// signatures: C from `git_commit_header` literals, Rust from [`Header`]s built by
/// `new`, copied, and changed through their handles, the run's pointer handed to C as
/// the options' array. After every commit the return codes, the new ids and the raw
/// commit objects read back from each side's odb are compared, and the bytes are
/// checked for each header's `<field> <value>` line, newlines continued by a space.
///
/// Documented divergences, outside these comparisons:
///
/// - A NULL `field` or `value` is refused by libgit2 (`-1`, `GIT_ERROR_INVALID`); a
///   [`Header`] has no NULL state, so no Rust input reaches that path.
/// - An empty Rust run passes a dangling, non-NULL pointer with a length of 0, where
///   C code passes NULL; libgit2 reads `extra_headers_len` first and never the
///   pointer, as the first step's comparison shows.
mod header {
    use std::ffi::{CStr, CString, c_int};
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::Header;

    use super::init;

    /// A fresh bare repository in its own temporary directory, removed on drop, with
    /// the empty tree, fixed signatures and the tip of the chain built so far.
    struct Side {
        dir: PathBuf,
        repo: *mut ffi::git_repository,
        tree: *mut ffi::git_tree,
        sig: *mut ffi::git_signature,
        tip: Option<*mut ffi::git_commit>,
    }

    impl Side {
        fn new(tag: &str) -> Self {
            init();
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-header-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: `repo` receives the repository this side frees; the path is
            // NUL-terminated.
            let rc = unsafe { ffi::git_repository_init(&mut repo, path.as_ptr(), 1) };
            assert_eq!(rc, 0);
            let (mut builder, mut tree_id, mut tree) =
                (ptr::null_mut(), ffi::git_oid::default(), ptr::null_mut());
            let mut sig = ptr::null_mut();
            // SAFETY: `repo` is open; each out-parameter receives an object this side
            // frees; the strings are NUL-terminated.
            unsafe {
                assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
                assert_eq!(ffi::git_treebuilder_write(&mut tree_id, builder), 0);
                ffi::git_treebuilder_free(builder);
                assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &tree_id), 0);
                assert_eq!(
                    ffi::git_signature_new(
                        &mut sig,
                        c"A U Thor".as_ptr(),
                        c"author@example.com".as_ptr(),
                        987654321,
                        90
                    ),
                    0
                );
            }
            Self {
                dir,
                repo,
                tree,
                sig,
                tip: None,
            }
        }

        /// Creates the next commit of the chain with `headers` as the options' extra
        /// headers, and returns the call's code, the new id and the raw object.
        fn commit(
            &mut self,
            headers: *const ffi::git_commit_header,
            len: usize,
        ) -> (c_int, String, Vec<u8>) {
            let opts = ffi::git_commit_create_ext_options {
                version: ffi::GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION,
                extra_headers: headers,
                extra_headers_len: len,
                ..Default::default()
            };
            let mut id = ffi::git_oid::default();
            let parents: Vec<*mut ffi::git_commit> = self.tip.into_iter().collect();
            // SAFETY: every object is this side's and live; `headers` addresses `len`
            // headers whose strings outlive the call (the caller's); `opts` is
            // initialised with this header's version.
            let rc = unsafe {
                ffi::git_commit_create_ext(
                    &mut id,
                    self.repo,
                    self.sig,
                    self.sig,
                    c"Extra headers\n".as_ptr(),
                    self.tree,
                    parents.len(),
                    parents.as_ptr(),
                    &opts,
                )
            };
            if rc != 0 {
                return (rc, String::new(), Vec::new());
            }
            let mut hex = [0u8; 41];
            let (mut odb, mut obj, mut commit) =
                (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            // SAFETY: `id` names the commit just written; each out-parameter receives an
            // object this side frees; `hex` has room for 40 digits and the NUL.
            let raw = unsafe {
                ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &id);
                assert_eq!(ffi::git_repository_odb(&mut odb, self.repo), 0);
                assert_eq!(ffi::git_odb_read(&mut obj, odb, &id), 0);
                let data = ffi::git_odb_object_data(obj).cast::<u8>();
                let raw = std::slice::from_raw_parts(data, ffi::git_odb_object_size(obj)).to_vec();
                ffi::git_odb_object_free(obj);
                ffi::git_odb_free(odb);
                assert_eq!(ffi::git_commit_lookup(&mut commit, self.repo, &id), 0);
                if let Some(old) = self.tip.replace(commit) {
                    ffi::git_commit_free(old);
                }
                raw
            };
            let hex = CStr::from_bytes_until_nul(&hex)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            (rc, hex, raw)
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            // SAFETY: each object is this side's and released once, the repository last.
            unsafe {
                if let Some(tip) = self.tip {
                    ffi::git_commit_free(tip);
                }
                ffi::git_signature_free(self.sig);
                ffi::git_tree_free(self.tree);
                ffi::git_repository_free(self.repo);
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn c_header(field: &CStr, value: &CStr) -> ffi::git_commit_header {
        ffi::git_commit_header {
            field: field.as_ptr(),
            value: value.as_ptr(),
        }
    }

    /// Compares one step's outcome and checks its header lines.
    fn step(
        c: &mut Side,
        rust: &mut Side,
        c_run: &[ffi::git_commit_header],
        rust_run: &[Header<'_>],
        lines: &[&str],
    ) {
        let expected = c.commit(
            if c_run.is_empty() {
                ptr::null()
            } else {
                c_run.as_ptr()
            },
            c_run.len(),
        );
        let got = rust.commit(rust_run.as_ptr().cast(), rust_run.len());
        assert_eq!(got.0, expected.0);
        assert_eq!(got.1, expected.1);
        assert_eq!(got.2, expected.2);
        assert_eq!(expected.0, 0);
        let text = String::from_utf8(expected.2).unwrap();
        let headers = text.split_once("\n\n").unwrap().0;
        let after_committer = headers.split_once("\ncommitter ").unwrap().1;
        let extra = after_committer
            .split_once('\n')
            .map_or("", |(_, rest)| rest);
        assert_eq!(extra, lines.join("\n"), "{text}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn git_commit_create_ext_reads_headers_as_c_arrays() {
        let (mut c, mut rust) = (Side::new("c"), Side::new("rust"));

        // No extra headers: NULL from C, an empty run from Rust.
        step(&mut c, &mut rust, &[], &[], &[]);

        // The headers of libgit2's own `can_add_extra_headers` test.
        let c_two = [
            c_header(c"line_one", c"First extra header"),
            c_header(c"line_two", c"Second extra header"),
        ];
        const TWO: [Header<'static>; 2] = [
            Header::new(c"line_one", c"First extra header"),
            Header::new(c"line_two", c"Second extra header"),
        ];
        step(
            &mut c,
            &mut rust,
            &c_two,
            &TWO,
            &[
                "line_one First extra header",
                "line_two Second extra header",
            ],
        );

        // Values with newlines, an empty value, and strings on the heap, set through
        // the handles of copies of the first run.
        let field = CString::new("x-heap").unwrap();
        let value = CString::new("one\ntwo\n").unwrap();
        let c_three = [
            c_header(&field, &value),
            c_header(c"line_two", c""),
            c_header(c"line_one", c"First extra header"),
        ];
        let mut three = [TWO[1], TWO[1], TWO[0]];
        {
            let mut m = three[0].as_mut();
            m.set_field(&field);
            m.set_value(&value);
        }
        three[1].as_mut().set_value(c"");
        step(
            &mut c,
            &mut rust,
            &c_three,
            &three,
            &[
                "x-heap one",
                " two",
                " ",
                "line_two ",
                "line_one First extra header",
            ],
        );

        // One header read from a longer run, and the same run read again.
        step(
            &mut c,
            &mut rust,
            &c_three[1..2],
            &three[1..2],
            &["line_two "],
        );
        step(
            &mut c,
            &mut rust,
            &c_three,
            &three,
            &[
                "x-heap one",
                " two",
                " ",
                "line_two ",
                "line_one First extra header",
            ],
        );
    }
}

/// Equivalence of the commit owner, [`CommitArc`], with the `git_commit` pointers C
/// code holds: [`Commit::lookup`] and [`Commit::lookup_prefix`] against
/// `git_commit_lookup` and `git_commit_lookup_prefix`, [`CommitRef::to_commit`] against
/// `git_commit_dup`, the drop against `git_commit_free`, and the object conversions
/// against C's unchecked casts guarded by `git_object_type`.
///
/// Each side opens its own `testrepo.git` and runs the same script: lookups of commit,
/// tree, blob, tag, missing and zero ids, in full and by prefixes of several lengths,
/// each kept or released at once. After every step the outcome is compared: the code,
/// class and message of a failure, or the found commit's id and type and which earlier
/// result of the same side it is the same commit as (the cache's sharing). Then every
/// kept commit is copied, converted to an object and back, and released in turn, and
/// the survivors compared after each release.
///
/// Documented divergence, outside these comparisons: an id of another type than the
/// repository's is refused in Rust before the call (`-1`, `GIT_ERROR_INVALID`), where
/// C formats it out of bounds; the module's `unit_tests` cover it.
mod commit_owner {
    use std::ffi::{CStr, CString, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{Commit, CommitArc, CommitRef};
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    use super::{init, open};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git"
    );

    const COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
    const PARENT: &str = "be3563ae3f795b2b4353bcce3a527ad0a4f7f644";
    const TREE: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
    const BLOB: &str = "a8233120f6ad708f843d861ce2b7228ec4e3dec6";
    const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
    const MISSING: &str = "dededededededededededededededededededede";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    /// What a step gave: the commit's id and type as C reads them, with the index of
    /// the earlier result of the same side that is the same commit, or the failure.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Found(String, ffi::git_object_t, Option<usize>),
        Failed(c_int, c_int, String),
    }

    fn view(p: *const ffi::git_object) -> (String, ffi::git_object_t) {
        let mut out = [0u8; 41];
        // SAFETY: `p` is a live object; its id is formatted into 41 writable bytes.
        unsafe {
            ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), ffi::git_object_id(p));
            let hex = CStr::from_bytes_until_nul(&out)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            (hex, ffi::git_object_type(p))
        }
    }

    fn c_failed(rc: c_int) -> Outcome {
        // SAFETY: after a failure the thread's error is set and live until the next
        // libgit2 call, after the copy.
        let (class, message) = unsafe {
            let e = ffi::git_error_last();
            (
                (*e).klass,
                CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
            )
        };
        Outcome::Failed(rc, class, message)
    }

    fn rust_failed(e: &RepositoryError) -> Outcome {
        Outcome::Failed(
            e.raw_code(),
            e.raw_class(),
            e.message().to_string_lossy().into_owned(),
        )
    }

    fn oid(hex: &str) -> ffi::git_oid {
        let mut id = ffi::git_oid::default();
        let hex = CString::new(hex).unwrap();
        // SAFETY: a writable oid and a NUL-terminated 40-digit hex string.
        let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        id
    }

    /// One step of the script: a hex id, a prefix length (`None`: the full id), and
    /// whether the result is kept.
    type Step = (&'static str, Option<usize>, bool);

    fn script() -> Vec<Step> {
        let mut steps = Vec::new();
        for hex in [COMMIT, TREE, BLOB, TAG, MISSING, ZERO, PARENT, COMMIT] {
            for (i, len) in [None, Some(40), Some(64), Some(7), Some(4), Some(3), Some(0)]
                .into_iter()
                .enumerate()
            {
                steps.push((hex, len, i % 2 == 0));
            }
        }
        steps
    }

    struct CSide {
        repo: *mut ffi::git_repository,
        held: Vec<*mut ffi::git_commit>,
    }

    impl CSide {
        fn step(&mut self, (hex, len, keep): Step) -> Outcome {
            let id = oid(hex);
            let mut out = ptr::null_mut();
            // SAFETY: a writable slot, a live repository and a live id of its type.
            let rc = unsafe {
                match len {
                    None => ffi::git_commit_lookup(&mut out, self.repo, &id),
                    Some(len) => ffi::git_commit_lookup_prefix(&mut out, self.repo, &id, len),
                }
            };
            if rc < 0 {
                return c_failed(rc);
            }
            let (hex, kind) = view(out.cast());
            let same = self.held.iter().position(|&h| h == out);
            if keep {
                self.held.push(out);
            } else {
                // SAFETY: the reference the lookup handed out, released once.
                unsafe { ffi::git_commit_free(out) };
            }
            Outcome::Found(hex, kind, same)
        }

        /// Copies, casts to an object and back, and releases the first commit held:
        /// the copy's view, and the survivors'.
        fn copy_cast_and_release(&mut self) -> (Outcome, Vec<(String, ffi::git_object_t)>) {
            let first = self.held.remove(0);
            let mut copy = ptr::null_mut();
            // SAFETY: a writable slot and a live commit, whose reference is held.
            assert_eq!(unsafe { ffi::git_commit_dup(&mut copy, first) }, 0);
            // SAFETY: the reference held, released once.
            unsafe { ffi::git_commit_free(first) };
            let object: *mut ffi::git_object = copy.cast();
            // SAFETY: `object` is the copy, a live object.
            let kind = unsafe { ffi::git_object_type(object) };
            assert_eq!(kind, ffi::GIT_OBJECT_COMMIT);
            let back: *mut ffi::git_commit = object.cast();
            let (hex, kind) = view(back.cast());
            let same = self.held.iter().position(|&h| h == back);
            // SAFETY: the copy's reference, released once, through the object.
            unsafe { ffi::git_object_free(object) };
            let rest = self.held.iter().map(|&h| view(h.cast())).collect();
            (Outcome::Found(hex, kind, same), rest)
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            for &commit in &self.held {
                // SAFETY: each held reference is released once, before the repository.
                unsafe { ffi::git_commit_free(commit) };
            }
            // SAFETY: the side's repository, freed once.
            unsafe { ffi::git_repository_free(self.repo) };
        }
    }

    fn rust_step<'r>(
        repo: &'r RepositoryBox,
        held: &mut Vec<CommitArc<'r>>,
        (hex, len, keep): Step,
    ) -> Outcome {
        let id = Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap();
        let result = match len {
            None => Commit::lookup(repo.as_ref(), id.as_ref()),
            Some(len) => Commit::lookup_prefix(repo.as_ref(), id.as_ref(), len),
        };
        let commit = match result {
            Ok(commit) => commit,
            Err(e) => return rust_failed(&e),
        };
        let (hex, kind) = view(commit.as_ref().as_object().to_object().as_c_ptr());
        let same = held.iter().position(|h| CommitArc::ptr_eq(h, &commit));
        if keep {
            held.push(commit);
        }
        Outcome::Found(hex, kind, same)
    }

    fn rust_copy_cast_and_release<'r>(
        held: &mut Vec<CommitArc<'r>>,
    ) -> (Outcome, Vec<(String, ffi::git_object_t)>) {
        let first = held.remove(0);
        let copy = first.as_ref().to_commit();
        drop(first);
        let object: ObjectArc<'r> = Commit::into_object(copy);
        let back = Commit::from_object(object).unwrap();
        let (hex, kind) = view(Commit::into_object(back.clone()).as_c_ptr());
        let same = held.iter().position(|h| CommitArc::ptr_eq(h, &back));
        drop(back);
        let rest = held
            .iter()
            .map(|h| view(h.as_ref().as_object().to_object().as_c_ptr()))
            .collect();
        (Outcome::Found(hex, kind, same), rest)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_copies_and_releases_match_c() {
        init();
        let mut c = CSide {
            repo: open(c"tests/resources/testrepo.git"),
            held: Vec::new(),
        };
        let rust = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        let mut held = Vec::new();
        for (i, step) in script().into_iter().enumerate() {
            let expected = c.step(step);
            let got = rust_step(&rust, &mut held, step);
            assert_eq!(got, expected, "step {i}: {step:?}");
        }
        assert_eq!(held.len(), c.held.len());
        while !c.held.is_empty() {
            let expected = c.copy_cast_and_release();
            assert_eq!(rust_copy_cast_and_release(&mut held), expected);
        }
    }

    /// Objects of every type, viewed and converted as commits: C's guard on
    /// `git_object_type` decides exactly as the conversions do, and a refused object
    /// comes back usable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn object_conversions_match_c_type_checks() {
        init();
        let c_repo = open(c"tests/resources/testrepo.git");
        let rust = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        for hex in [COMMIT, TREE, BLOB, TAG, PARENT] {
            let id = oid(hex);
            let mut object = ptr::null_mut();
            // SAFETY: a writable slot, a live repository and a live id.
            let rc =
                unsafe { ffi::git_object_lookup(&mut object, c_repo, &id, ffi::GIT_OBJECT_ANY) };
            assert_eq!(rc, 0);
            // SAFETY: a live object.
            let c_is_commit = unsafe { ffi::git_object_type(object) } == ffi::GIT_OBJECT_COMMIT;
            let c_view = view(object);
            // SAFETY: the lookup's reference, released once.
            unsafe { ffi::git_object_free(object) };

            let rust_id = Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap();
            let object = Object::lookup(rust.as_ref(), rust_id.as_ref(), None).unwrap();
            assert_eq!(
                CommitRef::from_object(object.as_ref()).is_some(),
                c_is_commit,
                "{hex}"
            );
            let converted = Commit::from_object(object);
            assert_eq!(converted.is_ok(), c_is_commit, "{hex}");
            let back = match converted {
                Ok(commit) => Commit::into_object(commit),
                Err(object) => object,
            };
            assert_eq!(view(back.as_c_ptr()), c_view, "{hex}");
        }
        // SAFETY: the C side's repository, freed once.
        unsafe { ffi::git_repository_free(c_repo) };
    }
}

/// Equivalence of the getters of [`CommitRef`] with the C accessors they wrap:
/// `git_commit_id`, `_owner`, `_message`, `_message_raw`, `_message_encoding`,
/// `_raw_header`, `_header_field`, `_summary`, `_time`, `_time_offset`, `_tree_id`,
/// `_parentcount` and `_parent_id`.
///
/// Each side opens its own copy of `testrepo.git` and of a fresh bare repository into
/// which it writes the same hand-made commits through its own object database: an
/// `encoding` header, a multi-line `gpgsig` header and a one-letter header, a header
/// whose last continuation line does not end, no message, a message of whitespace,
/// leading newlines and spaces, carriage returns and tabs in the first paragraph, two
/// parents and a negative zone. Both sides look up every commit and record, through
/// their own API, everything the getters give: ids, the owner's path, the strings, the
/// summary, the time, every parent id up to past the last and `u32::MAX`, and the value
/// or failure (code, class, message) of a list of header fields. The records are
/// compared after the lookups, again once every summary has been computed (now read
/// from the commit), and again through a copy of each commit after the original is
/// released.
///
/// Documented divergence, outside these comparisons: a header field holding a newline
/// that matches across a line end, which libgit2 copies with a negative length, is
/// refused by the wrapper as a field it does not find; `commit.rs`'s unit tests hold
/// it. Fields holding a newline that libgit2 reads safely (they match nothing) are
/// compared here.
mod getters {
    use std::ffi::{CStr, CString, c_int};
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{Commit, CommitArc, CommitRef};
    use libgit2::libgit2::oid::oid::{Oid, OidRef, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    use super::{init, open};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git"
    );

    /// Every commit of `testrepo.git`.
    const TESTREPO_COMMITS: [&str; 15] = [
        "d07b0f9a8c89f1d9e74dc4fce6421dec5ef8a659",
        "258f0e2a959a364e40ed6603d5d44fbb24765b10",
        "763d71aadf09a7951596c9746c024e7eece7c7af",
        "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        "e90810b8df3e80c413d903f631643c716887138d",
        "6dcf9bf7541ee10456529833502442f385010c3d",
        "a4a7dce85cf63874e984719f4fdd239f5145052f",
        "be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
        "c47800c7266a2be04c571c04d5a6614691ea99bd",
        "9fd738e8f7967c078dceed8190330fc8648ee56a",
        "4a202b346bb0fb0db7eff3cffeb3c70babbd2045",
        "41bc8c69075bbdb46c5c6f0566cc8cc5b46e8bd9",
        "5001298e0c09ad9c34e4249bc5801c75e9754fa5",
        "5b5b025afb0b4c913b4c338a42934a3863bf3644",
        "8496071c1b46c854b31185ea97743be6a8774479",
    ];

    const FIELDS: [&CStr; 14] = [
        c"tree",
        c"parent",
        c"author",
        c"committer",
        c"encoding",
        c"gpgsig",
        c"x",
        c"foo",
        c"",
        c"tre",
        c"parents",
        c"mergetag",
        c"zz\nzz",
        c"\n",
    ];

    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
    const ONE: &str = "1111111111111111111111111111111111111111";
    const TWO: &str = "2222222222222222222222222222222222222222";

    /// The raw contents of the hand-made commits.
    fn crafted() -> Vec<String> {
        let who = "A U Thor <a@example.com> 1700000000 +0130";
        let west = "W <w@example.com> 1 -1245";
        let head = format!("tree {EMPTY_TREE}\nauthor {who}\ncommitter {who}\n");
        vec![
            format!("{head}encoding ISO-8859-1\n\nLatin\n"),
            format!(
                "{head}gpgsig -----BEGIN-----\n line\n\n -----END-----\nx y\n\n\n\nfirst  line\n\tcontinued\n  \nbody\n"
            ),
            format!("{head}foo bar\n baz"),
            format!("{head}\n"),
            format!("{head}\n \t\n  \n"),
            format!("{head}\n\n\n  lead\r\n\tmore \r\n\nbody\n"),
            format!("{head}\na\r\nb\x0bc\x0cd\n\n"),
            format!(
                "tree {EMPTY_TREE}\nparent {ONE}\nparent {TWO}\nauthor {west}\ncommitter {west}\n\nmerge\n"
            ),
            head.clone(),
        ]
    }

    /// A fresh bare repository holding the hand-made commits, written through C, and
    /// their ids; removed on drop.
    struct Crafted {
        dir: PathBuf,
        ids: Vec<String>,
    }

    impl Crafted {
        fn new(side: &str) -> Self {
            init();
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-getters-equiv-{}-{side}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let (mut repo, mut odb) = (ptr::null_mut(), ptr::null_mut());
            let mut ids = Vec::new();
            // SAFETY: writable slots and a NUL-terminated path; the odb and repository
            // are released once, after the writes, each of which reads its data for the
            // call only.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, path.as_ptr(), 1), 0);
                assert_eq!(ffi::git_repository_odb(&mut odb, repo), 0);
                for raw in crafted() {
                    let mut id = ffi::git_oid::default();
                    let rc = ffi::git_odb_write(
                        &mut id,
                        odb,
                        raw.as_ptr().cast(),
                        raw.len(),
                        ffi::GIT_OBJECT_COMMIT,
                    );
                    assert_eq!(rc, 0);
                    ids.push(hex(&id));
                }
                ffi::git_odb_free(odb);
                ffi::git_repository_free(repo);
            }
            Self { dir, ids }
        }

        fn path(&self) -> CString {
            CString::new(self.dir.to_str().unwrap()).unwrap()
        }
    }

    impl Drop for Crafted {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn hex(id: *const ffi::git_oid) -> String {
        let mut out = [0u8; 41];
        // SAFETY: a live id formatted into 41 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn rust_hex(id: OidRef<'_>) -> String {
        id.to_hex_string().unwrap()
    }

    fn oid(hex: &str) -> ffi::git_oid {
        let mut id = ffi::git_oid::default();
        let hex = CString::new(hex).unwrap();
        // SAFETY: a writable oid and a NUL-terminated 40-digit hex string.
        let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        id
    }

    type Failure = (c_int, c_int, Vec<u8>);

    /// What one side observes of a commit.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Record {
        id: String,
        owner: Vec<u8>,
        message: Vec<u8>,
        raw: Vec<u8>,
        encoding: Option<Vec<u8>>,
        header: Vec<u8>,
        summary: Option<Vec<u8>>,
        time: (i64, c_int),
        tree: String,
        count: u32,
        parents: Vec<Option<String>>,
        fields: Vec<Result<Vec<u8>, Failure>>,
    }

    /// The parent indices probed: every one, one past the last, and `u32::MAX`.
    fn probes(count: u32) -> Vec<u32> {
        (0..=count + 1).chain([u32::MAX]).collect()
    }

    fn bytes(s: *const std::ffi::c_char) -> Vec<u8> {
        // SAFETY: the C getters' strings are NUL-terminated and live while the commit
        // is, which outlives this copy.
        unsafe { CStr::from_ptr(s) }.to_bytes().to_vec()
    }

    fn c_record(commit: *mut ffi::git_commit) -> Record {
        // SAFETY: `commit` is a live commit of a live repository, read through its
        // accessors; each string or id is copied at once, the header buffer disposed.
        unsafe {
            let encoding = ffi::git_commit_message_encoding(commit);
            let summary = ffi::git_commit_summary(commit);
            let count = ffi::git_commit_parentcount(commit);
            let parents = probes(count)
                .into_iter()
                .map(|n| {
                    let id = ffi::git_commit_parent_id(commit, n);
                    (!id.is_null()).then(|| hex(id))
                })
                .collect();
            let fields = FIELDS
                .iter()
                .map(|field| {
                    let mut buf = ffi::git_buf::default();
                    let rc = ffi::git_commit_header_field(&mut buf, commit, field.as_ptr());
                    let result = if rc < 0 {
                        let e = ffi::git_error_last();
                        Err((rc, (*e).klass, bytes((*e).message)))
                    } else {
                        Ok(bytes(buf.ptr))
                    };
                    ffi::git_buf_dispose(&mut buf);
                    result
                })
                .collect();
            Record {
                id: hex(ffi::git_commit_id(commit)),
                owner: bytes(ffi::git_repository_path(ffi::git_commit_owner(commit))),
                message: bytes(ffi::git_commit_message(commit)),
                raw: bytes(ffi::git_commit_message_raw(commit)),
                encoding: (!encoding.is_null()).then(|| bytes(encoding)),
                header: bytes(ffi::git_commit_raw_header(commit)),
                summary: (!summary.is_null()).then(|| bytes(summary)),
                time: (
                    ffi::git_commit_time(commit),
                    ffi::git_commit_time_offset(commit),
                ),
                tree: hex(ffi::git_commit_tree_id(commit)),
                count,
                parents,
                fields,
            }
        }
    }

    fn rust_record(commit: CommitRef<'_, '_>) -> Record {
        let count = commit.parent_count();
        Record {
            id: rust_hex(commit.id()),
            owner: commit.owner().path().unwrap().to_bytes().to_vec(),
            message: commit.message().to_bytes().to_vec(),
            raw: commit.message_raw().to_bytes().to_vec(),
            encoding: commit.message_encoding().map(|e| e.to_bytes().to_vec()),
            header: commit.raw_header().to_bytes().to_vec(),
            summary: commit.summary().ok().map(|s| s.to_bytes().to_vec()),
            time: (commit.time(), commit.time_offset()),
            tree: rust_hex(commit.tree_id()),
            count,
            parents: probes(count)
                .into_iter()
                .map(|n| commit.parent_id(n).map(rust_hex))
                .collect(),
            fields: FIELDS
                .iter()
                .map(|field| match commit.header_field(field) {
                    Ok(buf) => Ok(buf.as_ref().as_bytes().to_vec()),
                    Err(e) => Err((e.raw_code(), e.raw_class(), e.message().to_bytes().to_vec())),
                })
                .collect(),
        }
    }

    /// The owner paths differ by side (each opened its own copy): compare them as the
    /// side's own repository path.
    fn normalized(mut record: Record, own: &[u8]) -> Record {
        assert_eq!(record.owner, own);
        record.owner.clear();
        record
    }

    fn c_open(path: &CStr) -> *mut ffi::git_repository {
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
        repo
    }

    fn c_lookup(repo: *mut ffi::git_repository, hex: &str) -> *mut ffi::git_commit {
        let mut commit = ptr::null_mut();
        // SAFETY: a writable slot, a live repository and an id of its type.
        let rc = unsafe { ffi::git_commit_lookup(&mut commit, repo, &oid(hex)) };
        assert_eq!(rc, 0, "{hex}");
        commit
    }

    fn rust_lookup<'r>(repo: &'r RepositoryBox, hex: &str) -> CommitArc<'r> {
        let id = Oid::from_string(&CString::new(hex).unwrap(), OidType::Sha1).unwrap();
        Commit::lookup(repo.as_ref(), id.as_ref()).unwrap()
    }

    fn c_path(repo: *mut ffi::git_repository) -> Vec<u8> {
        // SAFETY: a live repository with a git directory.
        bytes(unsafe { ffi::git_repository_path(repo) })
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn getters_match_c_through_lookups_summaries_and_copies() {
        init();
        let (c_crafted, rust_crafted) = (Crafted::new("c"), Crafted::new("rust"));
        assert_eq!(c_crafted.ids, rust_crafted.ids);
        let c_repos = [
            open(c"tests/resources/testrepo.git"),
            c_open(&c_crafted.path()),
        ];
        let rust_repos = [
            Repository::open(&CString::new(TESTREPO).unwrap()).unwrap(),
            Repository::open(&rust_crafted.path()).unwrap(),
        ];
        let sets: [Vec<String>; 2] = [
            TESTREPO_COMMITS.iter().map(|s| s.to_string()).collect(),
            c_crafted.ids.clone(),
        ];
        for (side, ids) in sets.iter().enumerate() {
            let c_own = c_path(c_repos[side]);
            let rust_own = rust_repos[side]
                .as_ref()
                .path()
                .unwrap()
                .to_bytes()
                .to_vec();
            let c_commits: Vec<_> = ids.iter().map(|h| c_lookup(c_repos[side], h)).collect();
            let rust_commits: Vec<_> = ids
                .iter()
                .map(|h| rust_lookup(&rust_repos[side], h))
                .collect();
            for round in 0..2 {
                for (i, (&c, rust)) in c_commits.iter().zip(&rust_commits).enumerate() {
                    let expected = normalized(c_record(c), &c_own);
                    let got = normalized(rust_record(rust.as_ref()), &rust_own);
                    assert_eq!(
                        got, expected,
                        "set {side}, commit {i} ({}), round {round}",
                        ids[i]
                    );
                }
            }
            // Copies of each commit, read once the originals are released.
            let c_copies: Vec<_> = c_commits
                .into_iter()
                .map(|c| {
                    let mut copy = ptr::null_mut();
                    // SAFETY: a writable slot and a live commit; then its reference,
                    // released once, the copy keeping the commit.
                    unsafe {
                        assert_eq!(ffi::git_commit_dup(&mut copy, c), 0);
                        ffi::git_commit_free(c);
                    }
                    copy
                })
                .collect();
            let rust_copies: Vec<_> = rust_commits
                .into_iter()
                .map(|c| c.as_ref().to_commit())
                .collect();
            for (i, (&c, rust)) in c_copies.iter().zip(&rust_copies).enumerate() {
                let expected = normalized(c_record(c), &c_own);
                assert_eq!(
                    normalized(rust_record(rust.as_ref()), &rust_own),
                    expected,
                    "set {side}, copy {i}"
                );
                // SAFETY: the copy's reference, released once.
                unsafe { ffi::git_commit_free(c) };
            }
        }
        for repo in c_repos {
            // SAFETY: each C repository, freed once, after its commits.
            unsafe { ffi::git_repository_free(repo) };
        }
    }
}

/// Equivalence of the owners a commit derives, [`CommitRef::parent`],
/// [`CommitRef::nth_gen_ancestor`] and [`CommitRef::tree`], with `git_commit_parent`,
/// `git_commit_nth_gen_ancestor` and `git_commit_tree`.
///
/// Each side opens its own copies of three repositories: `testrepo.git` (merges with two
/// parents, roots, a seven-generation first-parent chain), `shallow.git` (whose boundary
/// commit is grafted parentless) and a fresh bare repository into which the side writes
/// the same hand-made objects through the object database: a commit whose tree is
/// missing, one whose tree is a blob, one whose parent is a blob, one whose parent is
/// missing, and a child of the first. A script then derives, from base commits looked up
/// on each side, every parent up to the third, ancestors of seven generation counts up to
/// `u32::MAX` and the tree, keeping every other result. After each step both sides record
/// the result's id and type, which earlier kept result (or the base) is the very same
/// object, or the failure's code, class and message, and the records are compared. Once
/// the script ends each side releases its base commits first and then reads every kept
/// result again.
mod navigation {
    use std::ffi::{CStr, CString, c_int};
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{Commit, CommitArc};
    use libgit2::libgit2::object::tree::TreeArc;
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    use super::{init, open};

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../");

    /// What a step derives from its base commit.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        Parent(u32),
        Ancestor(u32),
        Tree,
    }

    const OPS: [Op; 11] = [
        Op::Tree,
        Op::Parent(0),
        Op::Parent(1),
        Op::Parent(2),
        Op::Ancestor(0),
        Op::Ancestor(1),
        Op::Ancestor(2),
        Op::Ancestor(3),
        Op::Ancestor(6),
        Op::Ancestor(7),
        Op::Ancestor(u32::MAX),
    ];

    /// What a step gave: the result's id and type as C reads them, whether it is the
    /// base itself, and the index of the earlier kept result of the same side that is
    /// the same object; or the failure.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Found(String, ffi::git_object_t, bool, Option<usize>),
        Failed(c_int, c_int, String),
    }

    fn view(p: *const ffi::git_object) -> (String, ffi::git_object_t) {
        let mut out = [0u8; 41];
        // SAFETY: `p` is a live object; its id is formatted into 41 writable bytes.
        unsafe {
            ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), ffi::git_object_id(p));
            let hex = CStr::from_bytes_until_nul(&out)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            (hex, ffi::git_object_type(p))
        }
    }

    fn c_failed(rc: c_int) -> Outcome {
        // SAFETY: after a failure the thread's error is set and live until the next
        // libgit2 call, after the copy.
        let (class, message) = unsafe {
            let e = ffi::git_error_last();
            (
                (*e).klass,
                CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
            )
        };
        Outcome::Failed(rc, class, message)
    }

    fn rust_failed(e: &RepositoryError) -> Outcome {
        Outcome::Failed(
            e.raw_code(),
            e.raw_class(),
            e.message().to_string_lossy().into_owned(),
        )
    }

    fn oid(hex: &str) -> ffi::git_oid {
        let mut id = ffi::git_oid::default();
        let hex = CString::new(hex).unwrap();
        // SAFETY: a writable oid and a NUL-terminated 40-digit hex string.
        let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
        assert_eq!(rc, 0);
        id
    }

    fn hex(id: &ffi::git_oid) -> String {
        let mut out = [0u8; 41];
        // SAFETY: a live id formatted into 41 writable bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    const MISSING: &str = "dededededededededededededededededededede";
    const SIGNATURE: &str = "A U Thor <author@example.com> 1234567890 +0000";

    /// A fresh bare repository holding the hand-made objects, removed on drop; `bases`
    /// are the commits written, in order.
    struct Crafted {
        dir: PathBuf,
        repo: *mut ffi::git_repository,
        bases: Vec<String>,
    }

    impl Crafted {
        fn new(tag: &str) -> Self {
            init();
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-navigation-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: `repo` receives the repository this side frees; the path is
            // NUL-terminated.
            let rc = unsafe { ffi::git_repository_init(&mut repo, path.as_ptr(), 1) };
            assert_eq!(rc, 0);
            let mut odb = ptr::null_mut();
            // SAFETY: `repo` is open; `odb` receives one reference, freed below.
            assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, repo) }, 0);
            let write = |data: &str, kind: ffi::git_object_t| {
                let mut id = ffi::git_oid::default();
                // SAFETY: `odb` is live; `data` is read for the call only.
                let rc = unsafe {
                    ffi::git_odb_write(&mut id, odb, data.as_ptr().cast(), data.len(), kind)
                };
                assert_eq!(rc, 0);
                hex(&id)
            };
            let commit = |tree: &str, parents: &[&str], message: &str| {
                let mut raw = format!("tree {tree}\n");
                for parent in parents {
                    raw.push_str(&format!("parent {parent}\n"));
                }
                raw.push_str(&format!(
                    "author {SIGNATURE}\ncommitter {SIGNATURE}\n\n{message}\n"
                ));
                write(&raw, ffi::GIT_OBJECT_COMMIT)
            };
            let blob = write("not a tree\n", ffi::GIT_OBJECT_BLOB);
            let empty = write("", ffi::GIT_OBJECT_TREE);
            let treeless = commit(MISSING, &[], "missing tree");
            let bases = vec![
                commit(&blob, &[], "tree is a blob"),
                commit(&empty, &[&blob], "parent is a blob"),
                commit(&empty, &[MISSING], "parent is missing"),
                commit(&empty, &[&treeless], "child of a treeless commit"),
                treeless,
            ];
            // SAFETY: the reference taken above, released once.
            unsafe { ffi::git_odb_free(odb) };
            Self { dir, repo, bases }
        }
    }

    impl Drop for Crafted {
        fn drop(&mut self) {
            // SAFETY: the side's repository, freed once, after every object of it.
            unsafe { ffi::git_repository_free(self.repo) };
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const TESTREPO_BASES: [&str; 6] = [
        "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        "be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
        "a4a7dce85cf63874e984719f4fdd239f5145052f",
        "8496071c1b46c854b31185ea97743be6a8774479",
        "41bc8c69075bbdb46c5c6f0566cc8cc5b46e8bd9",
        "763d71aadf09a7951596c9746c024e7eece7c7af",
    ];
    const SHALLOW_BASES: [&str; 2] = [
        "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        "be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
    ];

    /// The C side: the repositories it opened, its base commits and the results it
    /// keeps.
    struct CSide {
        repos: Vec<*mut ffi::git_repository>,
        bases: Vec<*mut ffi::git_commit>,
        held: Vec<*mut ffi::git_object>,
    }

    impl CSide {
        fn base(&mut self, repo: *mut ffi::git_repository, hex: &str) {
            let mut commit = ptr::null_mut();
            // SAFETY: a writable slot, a live repository and a live id of its type.
            let rc = unsafe { ffi::git_commit_lookup(&mut commit, repo, &oid(hex)) };
            assert_eq!(rc, 0, "{hex}");
            self.bases.push(commit);
        }

        fn step(&mut self, op: Op, keep: bool) -> Outcome {
            let base = *self.bases.last().unwrap();
            let mut out: *mut ffi::git_object = ptr::null_mut();
            // SAFETY: a writable slot of the routine's type and a live commit, whose
            // repository is live and used on this thread only.
            let rc = unsafe {
                match op {
                    Op::Parent(n) => {
                        ffi::git_commit_parent(ptr::from_mut(&mut out).cast(), base, n)
                    }
                    Op::Ancestor(n) => {
                        ffi::git_commit_nth_gen_ancestor(ptr::from_mut(&mut out).cast(), base, n)
                    }
                    Op::Tree => ffi::git_commit_tree(ptr::from_mut(&mut out).cast(), base),
                }
            };
            if rc < 0 {
                return c_failed(rc);
            }
            let (hex, kind) = view(out);
            let same = self.held.iter().position(|&h| h == out);
            let is_base = out == base.cast();
            if keep {
                self.held.push(out);
            } else {
                // SAFETY: the reference the routine handed out, released once.
                unsafe { ffi::git_object_free(out) };
            }
            Outcome::Found(hex, kind, is_base, same)
        }

        /// Releases the bases, then reads every kept result.
        fn release_bases(&mut self) -> Vec<(String, ffi::git_object_t)> {
            for commit in self.bases.drain(..) {
                // SAFETY: each base's reference, released once.
                unsafe { ffi::git_commit_free(commit) };
            }
            self.held.iter().map(|&h| view(h)).collect()
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            for &object in &self.held {
                // SAFETY: each kept reference, released once, before the repositories.
                unsafe { ffi::git_object_free(object) };
            }
            for &repo in &self.repos {
                // SAFETY: each repository this side opened, freed once.
                unsafe { ffi::git_repository_free(repo) };
            }
        }
    }

    /// A result the Rust side keeps.
    enum Held<'r> {
        Commit(CommitArc<'r>),
        Tree(TreeArc<'r>),
    }

    impl Held<'_> {
        fn object(&self) -> *const ffi::git_object {
            match self {
                Held::Commit(c) => c.as_c_ptr().cast(),
                Held::Tree(t) => t.as_c_ptr().cast(),
            }
        }
    }

    fn rust_step<'r>(
        base: &CommitArc<'r>,
        held: &mut Vec<Held<'r>>,
        op: Op,
        keep: bool,
    ) -> Outcome {
        let base = base.as_ref();
        let result = match op {
            Op::Parent(n) => base.parent(n).map(Held::Commit),
            Op::Ancestor(n) => base.nth_gen_ancestor(n).map(Held::Commit),
            Op::Tree => base.tree().map(Held::Tree),
        };
        let derived = match result {
            Ok(derived) => derived,
            Err(e) => return rust_failed(&e),
        };
        let p = derived.object();
        let (hex, kind) = view(p);
        let same = held.iter().position(|h| h.object() == p);
        let is_base = p == base.as_object().to_object().as_c_ptr().cast_const();
        if keep {
            held.push(derived);
        }
        Outcome::Found(hex, kind, is_base, same)
    }

    fn rust_open(name: &str) -> RepositoryBox {
        Repository::open(&CString::new(format!("{ROOT}{name}")).unwrap()).unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn derived_commits_and_trees_match_c() {
        init();
        let c_crafted = Crafted::new("c");
        let rust_crafted = Crafted::new("rust");
        assert_eq!(c_crafted.bases, rust_crafted.bases);
        let mut c = CSide {
            repos: vec![
                open(c"tests/resources/testrepo.git"),
                open(c"tests/resources/shallow.git"),
            ],
            bases: Vec::new(),
            held: Vec::new(),
        };
        let rust_testrepo = rust_open("tests/resources/testrepo.git");
        let rust_shallow = rust_open("tests/resources/shallow.git");
        let path = CString::new(rust_crafted.dir.to_str().unwrap()).unwrap();
        let rust_crafted_repo = Repository::open(&path).unwrap();

        let mut bases = Vec::new();
        bases.extend(
            TESTREPO_BASES
                .iter()
                .map(|h| (c.repos[0], &rust_testrepo, h.to_string())),
        );
        bases.extend(
            SHALLOW_BASES
                .iter()
                .map(|h| (c.repos[1], &rust_shallow, h.to_string())),
        );
        bases.extend(
            c_crafted
                .bases
                .iter()
                .map(|h| (c_crafted.repo, &rust_crafted_repo, h.clone())),
        );

        let mut rust_bases = Vec::new();
        let mut held = Vec::new();
        let mut messages = Vec::new();
        let mut step = 0;
        for (repo, rust_repo, base) in &bases {
            c.base(*repo, base);
            let id = Oid::from_string(&CString::new(base.as_str()).unwrap(), OidType::Sha1);
            let commit = Commit::lookup(rust_repo.as_ref(), id.unwrap().as_ref()).unwrap();
            for op in OPS {
                let keep = step % 2 == 0;
                let expected = c.step(op, keep);
                let got = rust_step(&commit, &mut held, op, keep);
                assert_eq!(got, expected, "step {step}: {op:?} of {base}");
                if let Outcome::Failed(_, _, message) = expected {
                    messages.push(message);
                }
                step += 1;
            }
            rust_bases.push(commit);
        }
        let expected = c.release_bases();
        drop(rust_bases);
        let got: Vec<_> = held.iter().map(|h| view(h.object())).collect();
        assert_eq!(got, expected);
        drop(held);
        // Before `c_crafted`, whose repository the side's objects belong to.
        drop(c);
        // Every kind of failure was reached, on both sides alike.
        for expected in [
            "parent 0 does not exist",
            "parent 2 does not exist",
            "object not found - no match for id (dededededededededededededededededededede)",
            "the requested type does not match the type in the ODB",
        ] {
            assert!(
                messages.iter().any(|m| m == expected),
                "{expected}: {messages:?}"
            );
        }
    }
}

/// Equivalence of the commit moves, [`CommitArray::from_commits`],
/// [`CommitArrayMut::set_commits`] and [`CommitArrayMut::take`], with the C code that
/// builds a `git_commitarray` by hand (a buffer of libgit2's allocator filled with
/// commit references) and empties it into its own list before freeing the buffer.
///
/// Each side looks its commits up in its own `testrepo.git`, in the same order; after
/// every step of the sequence (build, take, rebuild in reverse, replace with a subset,
/// empty, dispose) both record the array's count and ids and the list taken out, and
/// the records are compared.
mod array_moves {
    use std::ffi::{CStr, CString};
    use std::mem::size_of;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{Commit, CommitArc, CommitArray};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;

    use super::{Record, c_record, hex, init, open, rust_hex, rust_record};

    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git"
    );

    const COMMITS: [&CStr; 4] = [
        c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        c"be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
        c"c47800c7266a2be04c571c04d5a6614691ea99bd",
        c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
    ];

    /// C's `from_commits`: a fresh libgit2 buffer owning `commits`' references.
    fn c_from(commits: Vec<*mut ffi::git_commit>) -> ffi::git_commitarray {
        if commits.is_empty() {
            return ffi::git_commitarray::default();
        }
        let size = commits.len() * size_of::<*mut ffi::git_commit>();
        // SAFETY: the routine ignores `backend`; libgit2 is initialised.
        let buf = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), size) }
            .cast::<*mut ffi::git_commit>();
        assert!(!buf.is_null());
        for (i, &commit) in commits.iter().enumerate() {
            // SAFETY: `buf` holds `commits.len()` pointer slots.
            unsafe { buf.add(i).write(commit) };
        }
        ffi::git_commitarray {
            commits: buf,
            count: commits.len(),
        }
    }

    /// C's `take`: the references moved out, the buffer freed, the struct emptied.
    fn c_take(array: &mut ffi::git_commitarray) -> Vec<*mut ffi::git_commit> {
        let taken = (0..array.count)
            // SAFETY: the array holds `count` live commit pointers.
            .map(|i| unsafe { *array.commits.add(i) })
            .collect();
        if !array.commits.is_null() {
            // SAFETY: the buffer came from `git_odb_backend_data_alloc`; its references
            // moved out above.
            unsafe {
                ffi::git_odb_backend_data_free(ptr::null_mut(), array.commits.cast_mut().cast())
            };
        }
        *array = ffi::git_commitarray::default();
        taken
    }

    fn c_ids(commits: &[*mut ffi::git_commit]) -> Vec<String> {
        commits.iter().map(|&c| hex(c)).collect()
    }

    fn rust_ids(commits: &[CommitArc<'_>]) -> Vec<String> {
        commits.iter().map(|c| rust_hex(c.as_ref())).collect()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn moves_match_hand_built_c_arrays() {
        init();
        let c_repo = open(c"tests/resources/testrepo.git");
        let rust_repo = Repository::open(&CString::new(TESTREPO).unwrap()).unwrap();
        let c_commits: Vec<_> = COMMITS
            .iter()
            .map(|hex| {
                let mut id = ffi::git_oid::default();
                // SAFETY: a writable oid and a NUL-terminated 40-digit hex string.
                let rc =
                    unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
                assert_eq!(rc, 0);
                let mut commit = ptr::null_mut();
                // SAFETY: a writable slot, a live repository and a live id; the
                // reference goes into the C array below.
                let rc = unsafe { ffi::git_commit_lookup(&mut commit, c_repo, &id) };
                assert_eq!(rc, 0);
                commit
            })
            .collect();
        let rust_commits: Vec<_> = COMMITS
            .iter()
            .map(|hex| {
                let id = Oid::from_string(hex, OidType::Sha1).unwrap();
                Commit::lookup(rust_repo.as_ref(), id.as_ref()).unwrap()
            })
            .collect();
        let compare = |c: &ffi::git_commitarray,
                       r: &libgit2::libgit2::object::commit::CommitArrayVal<'_>,
                       step: &str| {
            let expected: Record = c_record(c);
            assert_eq!(rust_record(r.as_ref()), expected, "{step}");
        };

        let mut c = c_from(c_commits);
        let mut rust = CommitArray::from_commits(rust_commits).unwrap();
        compare(&c, &rust, "build");

        let c_taken = c_take(&mut c);
        let rust_taken = rust.as_mut().take();
        compare(&c, &rust, "take");
        assert_eq!(rust_ids(&rust_taken), c_ids(&c_taken));

        let c_reversed: Vec<_> = c_taken.into_iter().rev().collect();
        let rust_reversed: Vec<_> = rust_taken.into_iter().rev().collect();
        c = c_from(c_reversed);
        rust.as_mut().set_commits(rust_reversed).unwrap();
        compare(&c, &rust, "rebuild reversed");

        // Replace with a subset: C releases what it held, then builds anew.
        let mut subset = c_take(&mut c);
        let keep = subset.remove(1);
        for commit in subset {
            // SAFETY: a reference taken out above, released once.
            unsafe { ffi::git_commit_free(commit) };
        }
        c = c_from(vec![keep]);
        let rust_keep = rust.as_ref().get(1).unwrap().to_commit();
        rust.as_mut().set_commits(vec![rust_keep]).unwrap();
        compare(&c, &rust, "replace");

        let c_one = c_take(&mut c);
        c = c_from(c_one);
        let rust_one = rust.as_mut().take();
        rust.as_mut().set_commits(rust_one).unwrap();
        compare(&c, &rust, "refill");

        // SAFETY: `c` owns its commit and buffer, released once.
        unsafe { ffi::git_commitarray_dispose(&mut c) };
        drop(rust.as_mut().take());
        let empty = CommitArray::from_commits(Vec::new()).unwrap();
        compare(&c, &empty, "dispose");
        drop((rust, empty));
        // SAFETY: the C side's repository, freed once, after its commits.
        unsafe { ffi::git_repository_free(c_repo) };
    }
}

/// Equivalence of [`CreateExtOptions`] with the `git_commit_create_ext_options` C code
/// builds: [`CreateExtOptions::new`] against `git_commit_create_ext_options_init`, and
/// a sequence of commits created through `git_commit_create_ext`, one side reading C
/// structs built for each step, the other Rust values set through their setters (the
/// reference to update, the encoding, the extra headers, and none of them), with a
/// [`SignatureCallback`] for the steps that sign, where C passes a C callback doing the
/// same: adding the same header, then refusing with the same error class and message
/// when the step's code is negative. The Rust side builds its options afresh for each
/// step, since they borrow the callback for as long as they live.
///
/// Each side owns a fresh bare repository and builds the same commits over the empty
/// tree with fixed signatures, a step's parent being the side's previous commit when
/// the step asks for one. After every step the return codes, the errors, the new ids,
/// the raw commit objects, the signing callback's calls and the target of the updated
/// reference are compared.
///
/// Documented divergences, outside these comparisons: `git_commit_create_ext_options_init`
/// refuses another version than `GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION` (`-1`), which
/// [`CreateExtOptions::new`] never passes; and an empty run of headers is stored as
/// NULL with a length of 0, as C code stores none.
mod create_ext_options {
    use std::cell::Cell;
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::path::PathBuf;
    use std::ptr;
    use std::rc::Rc;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::StreamError;
    use libgit2::libgit2::object::commit::{
        CommitBuilderMut, CreateExtOptions, Header, SignatureCallback,
    };
    use libgit2::libgit2::repository::repository::RepositoryRef;
    use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};

    use super::init;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_c_init() {
        let mut c = ffi::git_commit_create_ext_options {
            version: 0,
            update_ref: c"x".as_ptr(),
            extra_headers_len: 9,
            ..Default::default()
        };
        // SAFETY: a writable struct and the header's version.
        let rc = unsafe {
            ffi::git_commit_create_ext_options_init(
                &mut c,
                ffi::GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        let rust = CreateExtOptions::new();
        let r = rust.as_ref();
        assert_eq!(r.version(), c.version);
        assert_eq!(r.update_ref().is_none(), c.update_ref.is_null());
        assert_eq!(r.message_encoding().is_none(), c.message_encoding.is_null());
        assert_eq!(r.extra_headers().len(), c.extra_headers_len);
        assert_eq!(r.has_signer(), c.sign.is_some());
        assert!(c.payload.is_null());
    }

    /// The signing callback's state, one per side, reached through shared borrows.
    pub(super) struct Signer {
        pub(super) rc: Cell<c_int>,
        pub(super) calls: Cell<usize>,
    }

    /// Adds a fixed `gpgsig` header through the lent builder, counts the call and
    /// returns the payload's code.
    pub(super) unsafe extern "C" fn sign(
        builder: *mut ffi::git_commitbuilder,
        _: *mut ffi::git_repository,
        _: *const c_char,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: the payload is the side's live `Signer`, shared-borrowed.
        let signer = unsafe { &*payload.cast::<Signer>() };
        signer.calls.set(signer.calls.get() + 1);
        // SAFETY: the builder libgit2 lends for the call, and NUL-terminated strings.
        let rc = unsafe {
            ffi::git_commitbuilder_add_header(
                builder,
                c"gpgsig".as_ptr(),
                c"--sig--\nline".as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        let rc = signer.rc.get();
        if rc < 0 {
            // SAFETY: a named class and a NUL-terminated message, which libgit2 copies.
            let set = unsafe {
                ffi::git_error_set_str(ffi::GIT_ERROR_CALLBACK as c_int, REFUSED.as_ptr())
            };
            assert_eq!(set, 0);
        }
        rc
    }

    /// The error both sides' signing callbacks refuse with.
    const REFUSED: &CStr = c"signing refused";

    /// The Rust side's callback: what `sign` does, through the builder's handle, over
    /// the side's `Signer`.
    pub(super) fn rust_sign(
        signer: &Signer,
        mut builder: CommitBuilderMut<'_>,
    ) -> Result<(), StreamError> {
        signer.calls.set(signer.calls.get() + 1);
        builder
            .add_header(c"gpgsig", c"--sig--\nline")
            .map_err(StreamError::from)?;
        match signer.rc.get() {
            rc if rc >= 0 => Ok(()),
            rc => Err(StreamError::new(ErrorClass::Callback, REFUSED)
                .with_code(ErrorCode::from_raw(rc).unwrap())),
        }
    }

    /// One step: the options' fields, and whether the commit's parent is the side's
    /// previous commit.
    #[derive(Clone, Copy, Debug)]
    struct Step {
        update_ref: Option<&'static CStr>,
        encoding: Option<&'static CStr>,
        headers: &'static [(&'static CStr, &'static CStr)],
        sign: Option<c_int>,
        parent: bool,
    }

    const NONE: Step = Step {
        update_ref: None,
        encoding: None,
        headers: &[],
        sign: None,
        parent: false,
    };

    fn script() -> Vec<Step> {
        let main = Some(c"refs/heads/main");
        vec![
            NONE,
            Step {
                update_ref: main,
                ..NONE
            },
            Step {
                update_ref: main,
                parent: true,
                ..NONE
            },
            // The tip is not the first parent: GIT_EMODIFIED.
            Step {
                update_ref: main,
                ..NONE
            },
            Step {
                update_ref: Some(c"HEAD"),
                ..NONE
            },
            Step {
                update_ref: Some(c"HEAD"),
                parent: true,
                ..NONE
            },
            Step {
                encoding: Some(c"ISO-8859-1"),
                headers: &[(c"x-one", c"1"), (c"x-two", c"a\nb")],
                parent: true,
                ..NONE
            },
            Step {
                headers: &[(c"x-one", c"")],
                sign: Some(0),
                parent: true,
                ..NONE
            },
            Step {
                sign: Some(1),
                ..NONE
            },
            // `GIT_PASSTHROUGH` fails the call, whatever libgit2's header says.
            Step {
                sign: Some(-30),
                parent: true,
                ..NONE
            },
            // An invalid reference name fails before anything is built or signed.
            Step {
                update_ref: Some(c"refs/heads/bad..name"),
                sign: Some(0),
                parent: true,
                ..NONE
            },
            // A reference whose tip is not the first parent fails before signing.
            Step {
                sign: Some(-7),
                update_ref: main,
                parent: true,
                ..NONE
            },
            Step {
                sign: Some(-7),
                ..NONE
            },
            Step {
                update_ref: main,
                parent: true,
                ..NONE
            },
            NONE,
        ]
    }

    /// What a step gave: the code, the error or the id and raw commit, the callback's
    /// calls so far, and the updated reference's target.
    type Outcome = (c_int, String, String, usize, Option<String>);

    struct Side {
        dir: PathBuf,
        repo: *mut ffi::git_repository,
        tree: *mut ffi::git_tree,
        sig: *mut ffi::git_signature,
        tip: Option<*mut ffi::git_commit>,
        signer: Rc<Signer>,
    }

    impl Side {
        fn new(tag: &str) -> Self {
            init();
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-options-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let (mut repo, mut builder, mut tree_id, mut tree, mut sig) = (
                ptr::null_mut(),
                ptr::null_mut(),
                ffi::git_oid::default(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            // SAFETY: writable slots and NUL-terminated strings; each object is freed on
            // drop.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, path.as_ptr(), 1), 0);
                assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
                assert_eq!(ffi::git_treebuilder_write(&mut tree_id, builder), 0);
                ffi::git_treebuilder_free(builder);
                assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &tree_id), 0);
                assert_eq!(
                    ffi::git_signature_new(
                        &mut sig,
                        c"A U Thor".as_ptr(),
                        c"a@example.com".as_ptr(),
                        123456789,
                        60
                    ),
                    0
                );
            }
            let signer = Rc::new(Signer {
                rc: Cell::new(0),
                calls: Cell::new(0),
            });
            Self {
                dir,
                repo,
                tree,
                sig,
                tip: None,
                signer,
            }
        }

        fn payload(&self) -> *mut c_void {
            ptr::from_ref(&*self.signer).cast_mut().cast()
        }

        /// `git_commit_create_ext` with the options at `opts`, and what it gave.
        fn create(
            &mut self,
            step: Step,
            opts: *const ffi::git_commit_create_ext_options,
        ) -> Outcome {
            if let Some(rc) = step.sign {
                self.signer.rc.set(rc);
            }
            let parents: Vec<*mut ffi::git_commit> = if step.parent {
                self.tip.into_iter().collect()
            } else {
                Vec::new()
            };
            let mut id = ffi::git_oid::default();
            // SAFETY: the side's live objects; `opts` addresses live options whose
            // borrows, callback and payload outlive the call.
            let rc = unsafe {
                ffi::git_commit_create_ext(
                    &mut id,
                    self.repo,
                    self.sig,
                    self.sig,
                    c"options\n".as_ptr(),
                    self.tree,
                    parents.len(),
                    parents.as_ptr(),
                    opts,
                )
            };
            let target = step.update_ref.and_then(|name| self.target(name));
            let calls = self.signer.calls.get();
            if rc < 0 {
                // SAFETY: after a failure the thread's error is set, and copied at once.
                let message = unsafe { CStr::from_ptr((*ffi::git_error_last()).message) };
                return (
                    rc,
                    message.to_string_lossy().into_owned(),
                    String::new(),
                    calls,
                    target,
                );
            }
            let (mut odb, mut obj, mut commit) =
                (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            let mut hex = [0u8; 41];
            // SAFETY: the commit just written, read back and copied; the tip replaced
            // releases the previous one.
            let raw = unsafe {
                ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &id);
                assert_eq!(ffi::git_repository_odb(&mut odb, self.repo), 0);
                assert_eq!(ffi::git_odb_read(&mut obj, odb, &id), 0);
                let data = std::slice::from_raw_parts(
                    ffi::git_odb_object_data(obj).cast::<u8>(),
                    ffi::git_odb_object_size(obj),
                );
                let raw = String::from_utf8(data.to_vec()).unwrap();
                ffi::git_odb_object_free(obj);
                ffi::git_odb_free(odb);
                assert_eq!(ffi::git_commit_lookup(&mut commit, self.repo, &id), 0);
                if let Some(old) = self.tip.replace(commit) {
                    ffi::git_commit_free(old);
                }
                raw
            };
            let hex = CStr::from_bytes_until_nul(&hex)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            (rc, hex, raw, calls, target)
        }

        /// The id the reference `name` resolves to, through one symbolic hop, in hex.
        fn target(&self, name: &CStr) -> Option<String> {
            let mut reference = ptr::null_mut();
            // SAFETY: a live repository and NUL-terminated names; each reference is
            // released once, its target copied first.
            unsafe {
                if ffi::git_reference_lookup(&mut reference, self.repo, name.as_ptr()) != 0 {
                    return None;
                }
                if ffi::git_reference_type(reference) == ffi::GIT_REFERENCE_SYMBOLIC {
                    let next = CString::from(CStr::from_ptr(ffi::git_reference_symbolic_target(
                        reference,
                    )));
                    ffi::git_reference_free(reference);
                    return self.target(&next);
                }
                let mut hex = [0u8; 41];
                ffi::git_oid_tostr(
                    hex.as_mut_ptr().cast(),
                    hex.len(),
                    ffi::git_reference_target(reference),
                );
                ffi::git_reference_free(reference);
                Some(
                    CStr::from_bytes_until_nul(&hex)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                )
            }
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            // SAFETY: each object is released once, the repository last.
            unsafe {
                if let Some(tip) = self.tip {
                    ffi::git_commit_free(tip);
                }
                ffi::git_signature_free(self.sig);
                ffi::git_tree_free(self.tree);
                ffi::git_repository_free(self.repo);
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn commits_created_with_the_options_match_c() {
        let script = script();
        let (mut c, mut rust) = (Side::new("c"), Side::new("rust"));
        // The Rust side's header runs, built before the options that borrow them.
        let runs: Vec<Vec<Header<'static>>> = script
            .iter()
            .map(|step| {
                step.headers
                    .iter()
                    .map(|&(f, v)| Header::new(f, v))
                    .collect()
            })
            .collect();
        let rust_signer = Rc::clone(&rust.signer);
        let mut sign_rust = |builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            rust_sign(&rust_signer, builder)
        };
        for (i, (&step, run)) in script.iter().zip(&runs).enumerate() {
            let c_headers: Vec<ffi::git_commit_header> = step
                .headers
                .iter()
                .map(|&(f, v)| ffi::git_commit_header {
                    field: f.as_ptr(),
                    value: v.as_ptr(),
                })
                .collect();
            let c_opts = ffi::git_commit_create_ext_options {
                version: ffi::GIT_COMMIT_CREATE_EXT_OPTIONS_VERSION,
                update_ref: step.update_ref.map_or(ptr::null(), CStr::as_ptr),
                message_encoding: step.encoding.map_or(ptr::null(), CStr::as_ptr),
                extra_headers: if c_headers.is_empty() {
                    ptr::null()
                } else {
                    c_headers.as_ptr()
                },
                extra_headers_len: c_headers.len(),
                sign: step.sign.map(|_| sign as _),
                payload: if step.sign.is_some() {
                    c.payload()
                } else {
                    ptr::null_mut()
                },
            };
            let expected = c.create(step, &c_opts);

            let mut signer = SignatureCallback::new(&mut sign_rust);
            let mut options = CreateExtOptions::new();
            let mut m = options.as_mut();
            m.set_update_ref(step.update_ref);
            m.set_message_encoding(step.encoding);
            m.set_extra_headers(run);
            m.set_signer(step.sign.map(|_| &mut signer));
            let got = rust.create(step, ptr::from_ref(&options).cast());
            signer.resume_panic();
            assert_eq!(got, expected, "step {i}: {step:?}");
        }
        // Every signing step but the two refused before signing called its callback.
        assert_eq!(c.signer.calls.get(), 4);
        assert_eq!(rust.signer.calls.get(), 4);
    }
}

/// Equivalence of [`CreateOptions`] with the `git_commit_create_options` C code builds,
/// and of the routines committing on `HEAD` with it: [`CreateOptions::new`] against
/// `git_commit_create_options_init`, and a sequence of commits created on `HEAD` through
/// [`create_from_tree`] and [`create_from_stage`], and amended through
/// [`amend_from_tree`] and [`amend_from_stage`], against `git_commit_create_from_tree`,
/// `git_commit_create_from_stage`, `git_commit_amend_from_tree` and
/// `git_commit_amend_from_stage`. One side reads C structs built for each step (or NULL),
/// the other Rust values set through their setters (or `None`): the empty-commit flag,
/// the author and the committer (a fixed one, or none for the default), the encoding,
/// the extra headers and the signing callback (the `create_ext_options` module's, adding
/// the same header and refusing alike). The C side's signatures come from
/// `git_signature_new`; the Rust side lends a `SignatureView` as the author and a
/// `SignatureBox` as the committer. The Rust side builds its options afresh for each
/// step, since they borrow the callback for as long as they live.
///
/// Each side owns a fresh bare repository whose configuration names the default
/// identity, and the same two trees (the empty one, and one holding a blob). The stage
/// routines commit the repository's index, read from one of the trees before the step,
/// with a second entry added for a tree the object database does not hold yet. Before
/// some steps both sides write a `MERGE_HEAD` naming one of their earlier commits (the
/// new commit's second parent), remove it, or empty the configured identity (the
/// default signature then fails). After every step the return codes, the error classes
/// and messages, the raw commit objects, the commit `HEAD` names, the signing callback's
/// calls and whether that tree has been written are compared. The default identity is
/// signed at the current time, which may differ between the sides: its timestamps are
/// masked, and every commit id (in `parent` lines, and `HEAD`) is compared by its
/// position in the side's history rather than by value.
///
/// Documented divergences, outside these comparisons:
///
/// - `git_commit_create_options_init` refuses another version than
///   `GIT_COMMIT_CREATE_OPTIONS_VERSION` (`-1`), which [`CreateOptions::new`] never
///   passes; and an empty run of headers is stored as NULL with a length of 0, as C code
///   stores none.
/// - The creating routines take the parents from `git_repository_commit_parents`, which,
///   when `MERGE_HEAD` holds a line it cannot parse or that names no commit, frees the
///   commits it collected while the object cache still holds them (a use after free C
///   cannot be run into here), and takes `HEAD` at a tag or a tree as a commit, writing
///   that object as a parent. The wrappers take them from `commit_parents`: they fail
///   with `-1` and the error, `HEAD` at a tag with `GIT_EMODIFIED` (its commit is not the
///   tip `HEAD` names), and `HEAD` at a tree with `-1`. `ub_safe::commit` holds them.
/// - The wrappers refuse, before libgit2 runs, a signature libgit2 cannot write (lost
///   strings, an `i32::MIN` offset) and a tree of another repository, which C asserts on
///   only after diffing it (`create_from_tree`) or writes unchecked (`amend_from_tree`);
///   the unit tests of `commit.rs` hold them.
///
/// [`create_from_tree`]: libgit2::libgit2::object::commit::create_from_tree
/// [`create_from_stage`]: libgit2::libgit2::object::commit::create_from_stage
/// [`amend_from_tree`]: libgit2::libgit2::object::commit::amend_from_tree
/// [`amend_from_stage`]: libgit2::libgit2::object::commit::amend_from_stage
mod create_options {
    use std::ffi::{CStr, CString, c_int};
    use std::path::PathBuf;
    use std::ptr;
    use std::rc::Rc;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{Signature, SignatureView, Time};
    use libgit2::libgit2::object::commit::{
        CommitBuilderMut, CreateOptions, Header, SignatureCallback, amend_from_stage,
        amend_from_tree, create_from_stage, create_from_tree,
    };
    use libgit2::libgit2::object::tree::Tree;
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};

    use super::create_ext_options::{Signer, rust_sign, sign};
    use super::init;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_c_init() {
        let mut c = ffi::git_commit_create_options {
            version: 0,
            _bitfield_1: ffi::git_commit_create_options::new_bitfield_1(1),
            message_encoding: c"x".as_ptr(),
            extra_headers_len: 9,
            ..Default::default()
        };
        // SAFETY: a writable struct and the header's version.
        let rc = unsafe {
            ffi::git_commit_create_options_init(&mut c, ffi::GIT_COMMIT_CREATE_OPTIONS_VERSION)
        };
        assert_eq!(rc, 0);
        let rust = CreateOptions::new();
        let r = rust.as_ref();
        assert_eq!(r.version(), c.version);
        assert_eq!(r.allow_empty_commit(), c.allow_empty_commit() != 0);
        assert_eq!(r.author().is_none(), c.author.is_null());
        assert_eq!(r.committer().is_none(), c.committer.is_null());
        assert_eq!(r.message_encoding().is_none(), c.message_encoding.is_null());
        assert_eq!(r.extra_headers().len(), c.extra_headers_len);
        assert_eq!(r.has_signer(), c.sign.is_some());
        assert!(c.payload.is_null());
    }

    /// Which routine a step calls: create on `HEAD` with one of the side's trees, or
    /// amend `HEAD` with one and a new message (`None` keeps the amended one's); or the
    /// same with the index read from one of the trees, plus the extra entry when the
    /// flag is set.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        Create(usize),
        Amend(usize, Option<&'static CStr>),
        Stage(usize, bool),
        AmendStage(usize, bool, Option<&'static CStr>),
    }

    /// What both sides do to their repository before a step: nothing; write a
    /// `MERGE_HEAD` naming the commit at this position of their history; remove it; or
    /// empty the configured identity, which the default signature then refuses.
    #[derive(Clone, Copy, Debug)]
    enum Pre {
        Nothing,
        MergeHead(usize),
        NoMergeHead,
        BreakIdentity,
    }

    /// One step: the preparation, the routine and the options' fields; `true` signatures
    /// are the side's own, `false` leaves the field unset for the default. `null` passes
    /// no options at all (NULL, `None`), whatever the fields say.
    #[derive(Clone, Copy, Debug)]
    struct Step {
        pre: Pre,
        op: Op,
        null: bool,
        allow_empty: bool,
        author: bool,
        committer: bool,
        encoding: Option<&'static CStr>,
        headers: &'static [(&'static CStr, &'static CStr)],
        sign: Option<c_int>,
    }

    const BASE: Step = Step {
        pre: Pre::Nothing,
        op: Op::Create(0),
        null: false,
        allow_empty: false,
        author: true,
        committer: true,
        encoding: None,
        headers: &[],
        sign: None,
    };

    /// A header making a commit larger than the 4096 bytes libgit2's object cache keeps
    /// one of.
    const BIG: &[(&CStr, &CStr)] = {
        const VALUE: [u8; 5001] = {
            let mut value = [b'b'; 5001];
            value[5000] = 0;
            value
        };
        match CStr::from_bytes_with_nul(&VALUE) {
            Ok(value) => &[(c"x-big", value)],
            Err(_) => panic!("one NUL, at the end"),
        }
    };

    fn script() -> Vec<Step> {
        vec![
            // The empty tree on an unborn `HEAD` changes nothing.
            Step {
                author: false,
                committer: false,
                ..BASE
            },
            Step {
                allow_empty: true,
                ..BASE
            },
            Step {
                op: Op::Create(1),
                encoding: Some(c"ISO-8859-1"),
                headers: &[(c"x-one", c"1"), (c"x-two", c"a\nb")],
                ..BASE
            },
            // The tree `HEAD` already has.
            Step {
                op: Op::Create(1),
                ..BASE
            },
            // Allowed, with the default identity for both.
            Step {
                op: Op::Create(1),
                allow_empty: true,
                author: false,
                committer: false,
                ..BASE
            },
            // The amended commit's author, message and encoding; the given committer.
            Step {
                op: Op::Amend(1, None),
                author: false,
                ..BASE
            },
            Step {
                op: Op::Amend(0, Some(c"amended\n")),
                committer: false,
                encoding: Some(c"ISO-8859-1"),
                headers: &[(c"x-amend", c"")],
                sign: Some(0),
                ..BASE
            },
            // A change, refused by the signer before anything is written.
            Step {
                op: Op::Create(1),
                sign: Some(-7),
                ..BASE
            },
            // `GIT_PASSTHROUGH` fails the call too.
            Step {
                op: Op::Create(1),
                sign: Some(-30),
                ..BASE
            },
            // Without a new message the amended commit's encoding wins over the options'.
            Step {
                op: Op::Amend(1, None),
                encoding: Some(c"UTF-16"),
                sign: Some(1),
                ..BASE
            },
            // Amending ignores the empty-commit check.
            Step {
                op: Op::Amend(1, Some(c"again\n")),
                ..BASE
            },
            Step {
                op: Op::Create(0),
                allow_empty: true,
                headers: &[(c"x-empty", c"")],
                sign: Some(0),
                ..BASE
            },
            // The index `HEAD`'s tree holds: `GIT_EUNCHANGED`, where the tree routine
            // gives -1.
            Step {
                op: Op::Stage(0, false),
                ..BASE
            },
            // A refusal of the signer leaves no commit, but the index's tree has been
            // written by then.
            Step {
                op: Op::Stage(1, true),
                sign: Some(-7),
                ..BASE
            },
            Step {
                op: Op::Stage(1, false),
                author: false,
                committer: false,
                encoding: Some(c"ISO-8859-1"),
                headers: &[(c"x-stage", c"s")],
                sign: Some(0),
                ..BASE
            },
            Step {
                op: Op::Stage(1, false),
                ..BASE
            },
            Step {
                op: Op::Stage(1, false),
                allow_empty: true,
                ..BASE
            },
            // The amended commit's author, message and encoding.
            Step {
                op: Op::AmendStage(1, true, None),
                author: false,
                encoding: Some(c"UTF-16"),
                ..BASE
            },
            Step {
                op: Op::AmendStage(0, false, Some(c"stage amended\n")),
                sign: Some(-30),
                ..BASE
            },
            Step {
                op: Op::AmendStage(0, false, Some(c"stage amended\n")),
                committer: false,
                headers: &[(c"x-amend", c"stage")],
                sign: Some(0),
                ..BASE
            },
            // No options: the default identity for both, empty commits refused.
            Step {
                op: Op::Create(0),
                null: true,
                ..BASE
            },
            Step {
                op: Op::Create(1),
                null: true,
                ..BASE
            },
            Step {
                op: Op::Stage(1, false),
                null: true,
                ..BASE
            },
            Step {
                op: Op::Amend(0, None),
                null: true,
                ..BASE
            },
            Step {
                op: Op::AmendStage(1, false, Some(c"no options\n")),
                null: true,
                ..BASE
            },
            // During a merge: `MERGE_HEAD`'s commit is the second parent, for the tree and
            // the stage routines alike; an amend keeps the amended commit's parents.
            Step {
                pre: Pre::MergeHead(0),
                op: Op::Create(0),
                encoding: Some(c"ISO-8859-1"),
                ..BASE
            },
            Step {
                pre: Pre::MergeHead(2),
                op: Op::Stage(1, true),
                allow_empty: true,
                sign: Some(0),
                ..BASE
            },
            Step {
                op: Op::Amend(1, Some(c"amended merge\n")),
                ..BASE
            },
            // Nothing changes against `HEAD`, merge or not.
            Step {
                op: Op::Stage(1, false),
                ..BASE
            },
            Step {
                pre: Pre::NoMergeHead,
                op: Op::Create(1),
                allow_empty: true,
                author: false,
                ..BASE
            },
            // The default identity fails wherever it is needed, and only there.
            Step {
                pre: Pre::BreakIdentity,
                op: Op::Create(0),
                committer: false,
                ..BASE
            },
            Step {
                op: Op::Stage(0, false),
                author: false,
                ..BASE
            },
            Step {
                op: Op::Amend(0, None),
                author: false,
                ..BASE
            },
            Step {
                op: Op::AmendStage(1, false, None),
                committer: false,
                ..BASE
            },
            Step {
                op: Op::Create(1),
                null: true,
                ..BASE
            },
            Step {
                op: Op::Amend(0, Some(c"fixed identities\n")),
                ..BASE
            },
            // Commits too large to be cached: every amend without an author reads `HEAD`'s
            // commit anew, and writes its author.
            Step {
                op: Op::Create(1),
                allow_empty: true,
                headers: BIG,
                ..BASE
            },
            Step {
                op: Op::Amend(0, None),
                author: false,
                headers: BIG,
                ..BASE
            },
            Step {
                op: Op::AmendStage(1, false, None),
                author: false,
                headers: BIG,
                ..BASE
            },
            // No options with the default identity still broken: it fails after the peel.
            Step {
                op: Op::Amend(0, None),
                null: true,
                ..BASE
            },
        ]
    }

    /// What a step gave: the code, the error class and message (or the masked raw
    /// commit), the position of `HEAD`'s commit in the side's history, the callback's
    /// calls so far, and whether the tree of the index with the extra entry is written.
    type Outcome = (c_int, c_int, String, Option<usize>, usize, bool);

    /// The default identity both repositories' configurations name.
    const DEFAULT_IDENTITY: &str = "Default Identity <default@example.com>";

    struct Side {
        dir: PathBuf,
        /// Taken when the side drops, after the objects of it are released.
        repo: Option<RepositoryBox>,
        trees: [*mut ffi::git_tree; 2],
        tree_ids: [Oid; 2],
        /// The blob of the second tree, which the index's extra entry names too.
        blob: ffi::git_oid,
        /// The id of the tree of the second tree's index with the extra entry, which
        /// only the stage steps write.
        staged: ffi::git_oid,
        author: *mut ffi::git_signature,
        committer: *mut ffi::git_signature,
        history: Vec<String>,
        signer: Rc<Signer>,
    }

    impl Side {
        fn new(tag: &str) -> Self {
            init();
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-create-options-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: a writable slot and a NUL-terminated path; the repository is freed
            // at once, and opened again once its configuration names the identity.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, path.as_ptr(), 1), 0);
                ffi::git_repository_free(repo);
            }
            let config = dir.join("config");
            let mut text = std::fs::read_to_string(&config).unwrap();
            text.push_str("[user]\n\tname = Default Identity\n\temail = default@example.com\n");
            std::fs::write(&config, text).unwrap();
            let opened = Repository::open(&path).unwrap();
            let repo = opened.as_c_ptr();
            let mut trees = [ptr::null_mut(); 2];
            let (mut builder, mut tree_id, mut blob_id, mut entry) = (
                ptr::null_mut(),
                ffi::git_oid::default(),
                ffi::git_oid::default(),
                ptr::null(),
            );
            let (mut author, mut committer) = (ptr::null_mut(), ptr::null_mut());
            // SAFETY: writable slots, NUL-terminated strings and buffers of the given
            // lengths, and the live repository; the builder is freed once, the rest when
            // the side drops.
            unsafe {
                for (i, tree) in trees.iter_mut().enumerate() {
                    assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
                    if i == 1 {
                        let data = b"blob\n";
                        assert_eq!(
                            ffi::git_blob_create_from_buffer(
                                &mut blob_id,
                                repo,
                                data.as_ptr().cast(),
                                data.len()
                            ),
                            0
                        );
                        assert_eq!(
                            ffi::git_treebuilder_insert(
                                &mut entry,
                                builder,
                                c"file".as_ptr(),
                                &blob_id,
                                ffi::GIT_FILEMODE_BLOB
                            ),
                            0
                        );
                    }
                    assert_eq!(ffi::git_treebuilder_write(&mut tree_id, builder), 0);
                    ffi::git_treebuilder_free(builder);
                    assert_eq!(ffi::git_tree_lookup(tree, repo, &tree_id), 0);
                }
                assert_eq!(
                    ffi::git_signature_new(
                        &mut author,
                        c"A U Thor".as_ptr(),
                        c"author@example.com".as_ptr(),
                        123456789,
                        60
                    ),
                    0
                );
                assert_eq!(
                    ffi::git_signature_new(
                        &mut committer,
                        c"Com Mitter".as_ptr(),
                        c"committer@example.com".as_ptr(),
                        987654321,
                        -90
                    ),
                    0
                );
            }
            let tree_ids = trees.map(|t| {
                // SAFETY: the tree just looked up, live until the side drops; its id is
                // copied out.
                let id = unsafe { (*ffi::git_tree_id(t)).id };
                Oid::from_raw(&id[..20], OidType::Sha1).unwrap()
            });
            Self {
                dir,
                repo: Some(opened),
                trees,
                tree_ids,
                blob: blob_id,
                staged: ffi::git_oid::default(),
                author,
                committer,
                history: Vec::new(),
                signer: Rc::new(Signer {
                    rc: Default::default(),
                    calls: Default::default(),
                }),
            }
        }

        /// The repository, for the raw calls.
        fn c(&self) -> *mut ffi::git_repository {
            self.repo.as_ref().unwrap().as_c_ptr()
        }

        /// What both sides do before `step`.
        fn prepare(&self, step: Step) {
            if let Some(rc) = step.sign {
                self.signer.rc.set(rc);
            }
            let merge_head = self.dir.join("MERGE_HEAD");
            match step.pre {
                Pre::Nothing => {}
                Pre::MergeHead(at) => {
                    std::fs::write(merge_head, format!("{}\n", self.history[at])).unwrap()
                }
                Pre::NoMergeHead => std::fs::remove_file(merge_head).unwrap(),
                Pre::BreakIdentity => {
                    let config = self.dir.join("config");
                    let mut text = std::fs::read_to_string(&config).unwrap();
                    text.push_str("[user]\n\tname = \n");
                    std::fs::write(&config, text).unwrap();
                }
            }
            if let Op::Stage(tree, extra) | Op::AmendStage(tree, extra, _) = step.op {
                self.stage(tree, extra);
            }
        }

        /// The step's routine through the Rust wrapper with `options`, and what it gave.
        fn run_rust(&mut self, step: Step, options: Option<&mut CreateOptions<'_>>) -> Outcome {
            self.prepare(step);
            let repo = self.repo.as_ref().unwrap().as_ref();
            let tree = |i: usize| Tree::lookup(repo, self.tree_ids[i].as_ref()).unwrap();
            let options = options.map(|o| o.as_mut());
            let result = match step.op {
                Op::Create(t) => create_from_tree(repo, tree(t).as_ref(), c"options\n", options),
                Op::Amend(t, message) => amend_from_tree(repo, tree(t).as_ref(), message, options),
                Op::Stage(..) => create_from_stage(repo, c"options\n", options),
                Op::AmendStage(_, _, message) => amend_from_stage(repo, message, options),
            };
            match result {
                Ok(id) => {
                    let r = id.as_ref();
                    let id = ffi::git_oid {
                        type_: r.raw_kind(),
                        id: r.id(),
                    };
                    self.finish(0, Some(&id))
                }
                Err(e) => {
                    let error = (e.raw_class(), e.message().to_string_lossy().into_owned());
                    self.failed(e.raw_code(), error)
                }
            }
        }

        /// The step's routine with the options at `opts`, and what it gave.
        fn run(&mut self, step: Step, opts: *const ffi::git_commit_create_options) -> Outcome {
            self.prepare(step);
            let mut id = ffi::git_oid::default();
            // SAFETY: the side's live objects; `opts` addresses live options whose
            // borrows, callback and payload outlive the call; messages are NUL-terminated
            // or NULL where the routine allows it.
            let rc = unsafe {
                match step.op {
                    Op::Create(tree) => ffi::git_commit_create_from_tree(
                        &mut id,
                        self.c(),
                        self.trees[tree],
                        c"options\n".as_ptr(),
                        opts,
                    ),
                    Op::Amend(tree, message) => ffi::git_commit_amend_from_tree(
                        &mut id,
                        self.c(),
                        self.trees[tree],
                        message.map_or(ptr::null(), CStr::as_ptr),
                        opts,
                    ),
                    Op::Stage(..) => ffi::git_commit_create_from_stage(
                        &mut id,
                        self.c(),
                        c"options\n".as_ptr(),
                        opts,
                    ),
                    Op::AmendStage(_, _, message) => ffi::git_commit_amend_from_stage(
                        &mut id,
                        self.c(),
                        message.map_or(ptr::null(), CStr::as_ptr),
                        opts,
                    ),
                }
            };
            if rc < 0 {
                // SAFETY: after a failure the thread's error is set, and copied at once.
                let error = unsafe {
                    let e = ffi::git_error_last();
                    let message = CStr::from_ptr((*e).message);
                    ((*e).klass, message.to_string_lossy().into_owned())
                };
                return self.failed(rc, error);
            }
            self.finish(rc, Some(&id))
        }

        /// What a step that failed with `rc` and `(class, message)` gave.
        fn failed(&self, rc: c_int, (class, message): (c_int, String)) -> Outcome {
            let calls = self.signer.calls.get();
            (
                rc,
                class,
                message,
                self.head(),
                calls,
                self.staged_written(),
            )
        }

        /// What a step that wrote the commit `id` gave.
        fn finish(&mut self, rc: c_int, id: Option<&ffi::git_oid>) -> Outcome {
            let id = id.unwrap();
            let calls = self.signer.calls.get();
            let raw = self.raw(id);
            self.history.push(hex(id));
            let head = self.head();
            (rc, 0, self.mask(&raw), head, calls, self.staged_written())
        }

        /// Reads the second tree, or the empty one, into the repository's index, and adds
        /// the extra entry `other`, the second tree's blob, when `extra` is set.
        fn stage(&self, tree: usize, extra: bool) {
            let mut index = ptr::null_mut();
            // SAFETY: the live repository and its tree; the index reference is released
            // once; the entry is zeroed but for its mode, id and NUL-terminated path,
            // which libgit2 copies.
            unsafe {
                assert_eq!(ffi::git_repository_index(&mut index, self.c()), 0);
                assert_eq!(ffi::git_index_read_tree(index, self.trees[tree]), 0);
                if extra {
                    let mut entry: ffi::git_index_entry = std::mem::zeroed();
                    entry.mode = ffi::GIT_FILEMODE_BLOB;
                    entry.id = self.blob;
                    entry.path = c"other".as_ptr();
                    assert_eq!(ffi::git_index_add(index, &entry), 0);
                }
                ffi::git_index_free(index);
            }
        }

        /// The id of the tree of the index with the extra entry, written into this
        /// side's object database.
        fn write_staged(&self) -> ffi::git_oid {
            self.stage(1, true);
            let (mut index, mut id) = (ptr::null_mut(), ffi::git_oid::default());
            // SAFETY: the live repository; the index reference is released once.
            unsafe {
                assert_eq!(ffi::git_repository_index(&mut index, self.c()), 0);
                assert_eq!(ffi::git_index_write_tree(&mut id, index), 0);
                ffi::git_index_free(index);
            }
            id
        }

        /// Whether the object database holds the tree of the index with the extra entry.
        fn staged_written(&self) -> bool {
            let mut odb = ptr::null_mut();
            // SAFETY: the live repository and an id; the odb reference is released once.
            unsafe {
                assert_eq!(ffi::git_repository_odb(&mut odb, self.c()), 0);
                let found = ffi::git_odb_exists(odb, &self.staged) == 1;
                ffi::git_odb_free(odb);
                found
            }
        }

        /// The raw text of the commit `id`.
        fn raw(&self, id: &ffi::git_oid) -> String {
            let (mut odb, mut obj) = (ptr::null_mut(), ptr::null_mut());
            // SAFETY: the commit just written is read back and copied; each object is
            // released once.
            unsafe {
                assert_eq!(ffi::git_repository_odb(&mut odb, self.c()), 0);
                assert_eq!(ffi::git_odb_read(&mut obj, odb, id), 0);
                let data = std::slice::from_raw_parts(
                    ffi::git_odb_object_data(obj).cast::<u8>(),
                    ffi::git_odb_object_size(obj),
                );
                let raw = String::from_utf8(data.to_vec()).unwrap();
                ffi::git_odb_object_free(obj);
                ffi::git_odb_free(odb);
                raw
            }
        }

        /// `raw` with the default identity's timestamps masked and each parent named by
        /// its position in the side's history.
        fn mask(&self, raw: &str) -> String {
            raw.split_inclusive('\n')
                .map(|line| {
                    if let Some(parent) = line.strip_prefix("parent ") {
                        let at = self.position(parent.trim_end());
                        format!("parent #{at:?}\n")
                    } else if let Some(at) = line.find(DEFAULT_IDENTITY) {
                        format!("{}<time>\n", &line[..at + DEFAULT_IDENTITY.len() + 1])
                    } else {
                        line.to_owned()
                    }
                })
                .collect()
        }

        fn position(&self, hex: &str) -> Option<usize> {
            self.history.iter().position(|h| h == hex)
        }

        /// The position of the commit `HEAD` resolves to in the side's history.
        fn head(&self) -> Option<usize> {
            let mut head = ptr::null_mut();
            // SAFETY: the live repository; the reference is released once, its target
            // copied first.
            unsafe {
                if ffi::git_repository_head(&mut head, self.c()) != 0 {
                    return None;
                }
                let target = hex(&*ffi::git_reference_target(head));
                ffi::git_reference_free(head);
                self.position(&target)
            }
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            // SAFETY: each object is released once, the repository last.
            unsafe {
                ffi::git_signature_free(self.author);
                ffi::git_signature_free(self.committer);
                for tree in self.trees {
                    ffi::git_tree_free(tree);
                }
            }
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn hex(id: &ffi::git_oid) -> String {
        let mut hex = [0u8; 41];
        // SAFETY: a buffer of `hex.len()` bytes, which libgit2 NUL-terminates.
        unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), id) };
        CStr::from_bytes_until_nul(&hex)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn commits_created_and_amended_with_the_options_match_c() {
        let script = script();
        let (mut c, mut rust) = (Side::new("c"), Side::new("rust"));
        // Both sides' blobs are alike, and so is the tree of their index with the extra
        // entry: its id comes from a third repository, which writes it.
        let staged = Side::new("staged").write_staged();
        (c.staged, rust.staged) = (staged, staged);
        assert!(!c.staged_written() && !rust.staged_written());
        let author =
            SignatureView::new(c"A U Thor", c"author@example.com", Time::new(123456789, 60));
        let committer =
            Signature::new(c"Com Mitter", c"committer@example.com", 987654321, -90).unwrap();
        // The Rust side's header runs, built before the options that borrow them.
        let runs: Vec<Vec<Header<'static>>> = script
            .iter()
            .map(|step| {
                step.headers
                    .iter()
                    .map(|&(f, v)| Header::new(f, v))
                    .collect()
            })
            .collect();
        let rust_signer = Rc::clone(&rust.signer);
        let mut sign_rust = |builder: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &CStr| {
            rust_sign(&rust_signer, builder)
        };
        for (i, (&step, run)) in script.iter().zip(&runs).enumerate() {
            let c_headers: Vec<ffi::git_commit_header> = step
                .headers
                .iter()
                .map(|&(f, v)| ffi::git_commit_header {
                    field: f.as_ptr(),
                    value: v.as_ptr(),
                })
                .collect();
            let c_opts = ffi::git_commit_create_options {
                version: ffi::GIT_COMMIT_CREATE_OPTIONS_VERSION,
                _bitfield_1: ffi::git_commit_create_options::new_bitfield_1(
                    step.allow_empty.into(),
                ),
                author: if step.author { c.author } else { ptr::null() },
                committer: if step.committer {
                    c.committer
                } else {
                    ptr::null()
                },
                message_encoding: step.encoding.map_or(ptr::null(), CStr::as_ptr),
                extra_headers: if c_headers.is_empty() {
                    ptr::null()
                } else {
                    c_headers.as_ptr()
                },
                extra_headers_len: c_headers.len(),
                sign: step.sign.map(|_| sign as _),
                payload: if step.sign.is_some() {
                    Rc::as_ptr(&c.signer).cast_mut().cast()
                } else {
                    ptr::null_mut()
                },
            };
            let expected = c.run(step, if step.null { ptr::null() } else { &c_opts });

            let mut signer = SignatureCallback::new(&mut sign_rust);
            let mut options = CreateOptions::new();
            let mut m = options.as_mut();
            m.set_allow_empty_commit(step.allow_empty);
            m.set_author(step.author.then(|| author.as_ref()));
            m.set_committer(step.committer.then(|| committer.as_ref()));
            m.set_message_encoding(step.encoding);
            m.set_extra_headers(run);
            m.set_signer(step.sign.map(|_| &mut signer));
            let got = rust.run_rust(step, (!step.null).then_some(&mut options));
            assert_eq!(got, expected, "step {i}: {step:?}");
            if let (Op::Stage(_, true), Some(-7)) = (step.op, step.sign) {
                // Refused by the signer, yet the index's tree is in the object database.
                assert_eq!((expected.0, expected.5), (-7, true), "step {i}");
            }
        }
        // Every signing step reached its callback: none fails before signing.
        assert_eq!(c.signer.calls.get(), 10);
        assert_eq!(rust.signer.calls.get(), 10);
        assert_eq!((c.history.len(), rust.history.len()), (24, 24));
        assert!(c.staged_written() && rust.staged_written());
    }
}

/// Equivalence of `create_with_signature` and `extract_signature` with
/// `git_commit_create_with_signature` and `git_commit_extract_signature`.
///
/// Each side has its own bare repository holding the empty tree, and runs one script:
/// commits written unsigned and signed (default and custom fields, multi-line
/// signatures, a parent), contents that do not parse, signed contents without a blank
/// line (unparsable, and parsable), a missing tree; then every written commit's
/// signature extracted under each field (a newline among them, which matches the blank
/// line before the message), from commits with no, one and two signature headers, from
/// a blob and from a missing id. After every step the ids, the two buffers, or the codes, error classes
/// and messages are compared.
///
/// libgit2 matches the field as a prefix of each header line, so a header whose name
/// only starts with the field (`gpgsig-sha256` for `gpgsig`), placed before the one
/// taken, loses the field's bytes from the signed data: a C defect the wrapper keeps,
/// documented on it, and compared here like the rest.
///
/// Documented divergences, outside these comparisons: an id of another type than the
/// repository's (which libgit2 formats out of bounds) and an empty field (which it scans
/// for forever) are refused by the wrapper; the unit tests of `commit.rs` hold the
/// refusals.
mod signatures {
    use std::ffi::{CStr, CString, c_int};
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{create_with_signature, extract_signature};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    use super::init;

    type Failure = (c_int, c_int, String);

    const TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    fn content(parents: &[&str], extra: &str, message: &str) -> String {
        let mut s = format!("tree {TREE}\n");
        for p in parents {
            s += &format!("parent {p}\n");
        }
        s + "author A U Thor <a@example.com> 1700000000 +0100\n\
             committer C O Mitter <c@example.com> 1700000001 -0200\n"
            + extra
            + "\n"
            + message
    }

    struct Side {
        dir: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Side {
        fn new(tag: &str) -> Self {
            init();
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-sig-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let repo =
                Repository::init(&CString::new(dir.to_str().unwrap()).unwrap(), true).unwrap();
            let mut tree = ffi::git_oid::default();
            let mut odb = ptr::null_mut();
            // SAFETY: writable slots and the live repository; an empty tree is written from
            // a live (non-NULL) empty buffer, and the odb reference released once.
            unsafe {
                assert_eq!(ffi::git_repository_odb(&mut odb, repo.as_c_ptr()), 0);
                assert_eq!(
                    ffi::git_odb_write(
                        &mut tree,
                        odb,
                        c"".as_ptr().cast(),
                        0,
                        ffi::GIT_OBJECT_TREE
                    ),
                    0
                );
                ffi::git_odb_free(odb);
            }
            Self {
                dir,
                repo: Some(repo),
            }
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn c_failure(code: c_int) -> Failure {
        // SAFETY: after init `git_error_last` never returns NULL; its message is
        // NUL-terminated, copied before the next libgit2 call.
        unsafe {
            let e = ffi::git_error_last();
            (
                code,
                (*e).klass,
                CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
            )
        }
    }

    fn opt(s: Option<&str>) -> Option<CString> {
        s.map(|s| CString::new(s).unwrap())
    }

    fn c_create(
        side: &Side,
        text: &str,
        sig: Option<&str>,
        field: Option<&str>,
    ) -> Result<[u8; 20], Failure> {
        let (text, sig, field) = (CString::new(text).unwrap(), opt(sig), opt(field));
        let mut id = ffi::git_oid::default();
        // SAFETY: a writable id, the live repository, NUL-terminated or NULL strings.
        let rc = unsafe {
            ffi::git_commit_create_with_signature(
                &mut id,
                side.repo.as_ref().unwrap().as_c_ptr(),
                text.as_ptr(),
                sig.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
                field.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            )
        };
        if rc < 0 {
            Err(c_failure(rc))
        } else {
            Ok(id.id[..20].try_into().unwrap())
        }
    }

    fn rust_create(
        side: &Side,
        text: &str,
        sig: Option<&str>,
        field: Option<&str>,
    ) -> Result<[u8; 20], Failure> {
        let (text, sig, field) = (CString::new(text).unwrap(), opt(sig), opt(field));
        let repo = side.repo.as_ref().unwrap().as_ref();
        create_with_signature(repo, &text, sig.as_deref(), field.as_deref())
            .map(|id| id.as_ref().id()[..20].try_into().unwrap())
            .map_err(|e| {
                (
                    e.raw_code(),
                    e.raw_class(),
                    e.message().to_string_lossy().into_owned(),
                )
            })
    }

    type Extracted = Result<(Vec<u8>, Vec<u8>), Failure>;

    fn c_extract(side: &Side, id: [u8; 20], field: Option<&str>) -> Extracted {
        let field = opt(field);
        let mut oid = ffi::git_oid::default();
        oid.id[..20].copy_from_slice(&id);
        oid.type_ = ffi::GIT_OID_SHA1 as _;
        let (mut sig, mut data) = (ffi::git_buf::default(), ffi::git_buf::default());
        // SAFETY: two zeroed buffers, the live repository, a live SHA-1 id, and a NULL
        // or non-empty field; each buffer is disposed once after its copy.
        unsafe {
            let rc = ffi::git_commit_extract_signature(
                &mut sig,
                &mut data,
                side.repo.as_ref().unwrap().as_c_ptr(),
                &mut oid,
                field.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            );
            let copy = |b: &ffi::git_buf| {
                if b.ptr.is_null() {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(b.ptr.cast::<u8>(), b.size).to_vec()
                }
            };
            let out = if rc < 0 {
                Err(c_failure(rc))
            } else {
                Ok((copy(&sig), copy(&data)))
            };
            ffi::git_buf_dispose(&mut sig);
            ffi::git_buf_dispose(&mut data);
            out
        }
    }

    fn rust_extract(side: &Side, id: [u8; 20], field: Option<&str>) -> Extracted {
        let field = opt(field);
        let oid = Oid::from_raw(&id, OidType::Sha1).unwrap();
        let repo = side.repo.as_ref().unwrap().as_ref();
        extract_signature(repo, oid.as_ref(), field.as_deref())
            .map(|(s, d)| {
                (
                    s.as_ref().as_bytes().to_vec(),
                    d.as_ref().as_bytes().to_vec(),
                )
            })
            .map_err(|e| {
                (
                    e.raw_code(),
                    e.raw_class(),
                    e.message().to_string_lossy().into_owned(),
                )
            })
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn signatures_match_c() {
        let (c, rust) = (Side::new("c"), Side::new("rust"));
        let root = c_create(&c, &content(&[], "", "root\n"), None, None).unwrap();
        let root_hex: String = root.iter().map(|b| format!("{b:02x}")).collect();
        let sig = "-----BEGIN PGP SIGNATURE-----\n\nabc\ndef\n-----END PGP SIGNATURE-----";
        let creates: Vec<(String, Option<&str>, Option<&str>)> = vec![
            (content(&[], "", "root\n"), None, None),
            (content(&[], "", "signed\n"), Some(sig), None),
            (
                content(&[&root_hex], "", "child\n"),
                Some("one line"),
                Some("x-sig"),
            ),
            (
                content(&[&root_hex], "encoding UTF-8\n", ""),
                Some(sig),
                Some("gpgsig"),
            ),
            (
                content(&[], "gpgsig-sha256 other\n", "both\n"),
                Some(sig),
                None,
            ),
            (
                content(&[], "gpgsig first\n", "twice\n"),
                Some("second"),
                None,
            ),
            (content(&[], "", "m\n"), Some(""), Some("")),
            (content(&[], "", "m\n"), Some("trailing newline\n"), None),
            (
                content(&["dededededededededededededededededededede"], "", "m\n"),
                None,
                None,
            ),
            ("tree xyz\n".into(), Some(sig), None),
            (String::new(), None, None),
            (
                content(&[], "", "x")
                    .split_once("\n\n")
                    .unwrap()
                    .0
                    .to_owned(),
                Some(sig),
                None,
            ),
            // Contents that parse but have no blank line: signing them is malformed.
            (
                content(&[], "", "x")
                    .split_once("\n\n")
                    .unwrap()
                    .0
                    .to_owned()
                    + "\n",
                Some(sig),
                None,
            ),
            // A message whose first line starts with a space and does not end: the
            // field `\n` takes the blank line before it as a header, which is malformed.
            (content(&[], "", " unterminated"), None, None),
            // A signature header whose continuation never ends: extracting it is malformed.
            (
                content(&[], "", "x")
                    .split_once("\n\n")
                    .unwrap()
                    .0
                    .to_owned()
                    + "\ngpgsig a\n b",
                None,
                None,
            ),
        ];
        let mut written = vec![root];
        for (text, sig, field) in &creates {
            let c_out = c_create(&c, text, *sig, *field);
            assert_eq!(rust_create(&rust, text, *sig, *field), c_out, "{text:?}");
            written.extend(c_out.ok());
        }
        let blob = {
            let mut id = ffi::git_oid::default();
            // SAFETY: a writable id, the live repository, a live buffer.
            let rc = unsafe {
                ffi::git_blob_create_from_buffer(
                    &mut id,
                    rust.repo.as_ref().unwrap().as_c_ptr(),
                    c"b".as_ptr().cast(),
                    1,
                )
            };
            assert_eq!(rc, 0);
            // SAFETY: as above, on the C side's repository.
            let rc = unsafe {
                ffi::git_blob_create_from_buffer(
                    &mut id,
                    c.repo.as_ref().unwrap().as_c_ptr(),
                    c"b".as_ptr().cast(),
                    1,
                )
            };
            assert_eq!(rc, 0);
            id.id[..20].try_into().unwrap()
        };
        written.push(blob);
        written.push([0xde; 20]);
        let mut found = 0;
        for &id in &written {
            for field in [
                None,
                Some("gpgsig"),
                Some("x-sig"),
                Some("gpg"),
                Some("gpgsig-sha256"),
                Some("encoding"),
                Some("\n"),
            ] {
                let c_out = c_extract(&c, id, field);
                found += usize::from(c_out.is_ok());
                assert_eq!(rust_extract(&rust, id, field), c_out, "{id:x?} {field:?}");
            }
        }
        assert!(found >= 6, "{found}");
    }
}

/// The calling thread's last error class and message.
fn c_last() -> (c_int, String) {
    // SAFETY: `git_error_last` never returns NULL after init, and its message is a
    // NUL-terminated string, live until the next libgit2 call on this thread, after the
    // copy.
    unsafe {
        let e = ffi::git_error_last();
        (
            (*e).klass,
            CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
        )
    }
}

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process.
fn isolated(test: &str, body: fn()) {
    let name = test.strip_prefix("equiv::").unwrap_or(test);
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

/// Reading a commit: [`CommitRef::author`], [`CommitRef::committer`] and
/// [`CommitRef::body`], and the `_with_mailmap` copies, against `git_commit_author`,
/// `git_commit_committer`, `git_commit_body`, `git_commit_author_with_mailmap` and
/// `git_commit_committer_with_mailmap`.
///
/// Each side opens its own `testrepo.git` and looks every commit of a list up; both
/// read each signature (name, email, seconds, offset, sign), the body (twice: libgit2
/// computes it on the first call and keeps it), and the copies a mailmap of their own
/// resolves (or the failure), comparing after every commit.
///
/// [`CommitRef::author`]: libgit2::libgit2::object::commit::CommitRef::author
/// [`CommitRef::committer`]: libgit2::libgit2::object::commit::CommitRef::committer
/// [`CommitRef::body`]: libgit2::libgit2::object::commit::CommitRef::body
mod reading {
    use std::ffi::{CStr, CString, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::SignatureRef;
    use libgit2::libgit2::object::commit::Commit;
    use libgit2::libgit2::object::mailmap::Mailmap;
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::Repository;

    use super::{TESTREPO, open};

    /// Commits of `testrepo.git`: merges, roots, one-line and multi-paragraph messages.
    const COMMITS: [&str; 8] = [
        "a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
        "be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
        "c47800c7266a2be04c571c04d5a6614691ea99bd",
        "9fd738e8f7967c078dceed8190330fc8648ee56a",
        "4a202b346bb0fb0db7eff3cffeb3c70babbd2045",
        "5b5b025afb0b4c913b4c338a42934a3863bf3644",
        "8496071c1b46c854b31185ea97743be6a8774479",
        "e90810b8df3e80c413d903f631643c716887138d",
    ];

    const MAILMAP: &[u8] = b"Mapped Name <mapped@example.com> <schacon@gmail.com>\n\
        <mapped-only@example.com> <tanoku@gmail.com>\n";

    type Sig = (String, String, i64, c_int, u8);

    fn c_sig(s: *const ffi::git_signature) -> Sig {
        // SAFETY: a live signature with both strings, read and copied at once.
        unsafe {
            (
                CStr::from_ptr((*s).name).to_string_lossy().into_owned(),
                CStr::from_ptr((*s).email).to_string_lossy().into_owned(),
                (*s).when.time,
                (*s).when.offset,
                (*s).when.sign as u8,
            )
        }
    }

    fn rust_sig(s: SignatureRef<'_>) -> Sig {
        let when = s.when();
        (
            s.name().unwrap().to_string_lossy().into_owned(),
            s.email().unwrap().to_string_lossy().into_owned(),
            when.seconds(),
            when.offset_minutes(),
            when.sign(),
        )
    }

    /// What a side reads of a commit: both signatures, the body twice, and both mailmap
    /// copies (or the failure's code, class and message).
    type View = (
        Sig,
        Sig,
        Option<String>,
        Option<String>,
        Vec<Result<Sig, (c_int, c_int, String)>>,
    );

    fn c_view(commit: *mut ffi::git_commit, mailmap: *const ffi::git_mailmap) -> View {
        let body = || {
            // SAFETY: the live commit; the body is NULL or a string the commit keeps.
            let b = unsafe { ffi::git_commit_body(commit) };
            // SAFETY: as above, copied at once.
            (!b.is_null()).then(|| unsafe { CStr::from_ptr(b) }.to_string_lossy().into_owned())
        };
        let mut copies = Vec::new();
        for map in [ptr::null(), mailmap] {
            for author in [true, false] {
                let mut out = ptr::null_mut();
                // SAFETY: a writable slot, the live commit and a NULL or live mailmap.
                let rc = unsafe {
                    if author {
                        ffi::git_commit_author_with_mailmap(&mut out, commit, map)
                    } else {
                        ffi::git_commit_committer_with_mailmap(&mut out, commit, map)
                    }
                };
                copies.push(if rc < 0 {
                    let (class, message) = super::c_last();
                    Err((rc, class, message))
                } else {
                    let sig = c_sig(out);
                    // SAFETY: the fresh copy, released once.
                    unsafe { ffi::git_signature_free(out) };
                    Ok(sig)
                });
            }
        }
        // SAFETY: the live commit; the getter only reads it.
        let author = unsafe { ffi::git_commit_author(commit) };
        // SAFETY: as above.
        let committer = unsafe { ffi::git_commit_committer(commit) };
        (c_sig(author), c_sig(committer), body(), body(), copies)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reads_match() {
        let c_repo = open(TESTREPO);
        let mut c_mailmap = ptr::null_mut();
        // SAFETY: a writable slot and `MAILMAP.len()` readable bytes.
        let rc = unsafe {
            ffi::git_mailmap_from_buffer(&mut c_mailmap, MAILMAP.as_ptr().cast(), MAILMAP.len())
        };
        assert_eq!(rc, 0);
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../");
        let path = CString::new(format!("{root}{}", TESTREPO.to_str().unwrap())).unwrap();
        let rust = Repository::open(&path).unwrap();
        let mailmap = Mailmap::from_buffer(MAILMAP).unwrap();
        for hex in COMMITS {
            let mut raw = ffi::git_oid::default();
            let mut commit = ptr::null_mut();
            let c_hex = CString::new(hex).unwrap();
            // SAFETY: writable slots, a 40-digit id and the live repository.
            unsafe {
                assert_eq!(
                    ffi::git_oid_from_string(&mut raw, c_hex.as_ptr(), ffi::GIT_OID_SHA1),
                    0
                );
                assert_eq!(ffi::git_commit_lookup(&mut commit, c_repo, &raw), 0);
            }
            let expected = c_view(commit, c_mailmap);
            // SAFETY: the commit, released once.
            unsafe { ffi::git_commit_free(commit) };

            let id = Oid::from_string(&c_hex, OidType::Sha1).unwrap();
            let owner = Commit::lookup(rust.as_ref(), id.as_ref()).unwrap();
            let r = owner.as_ref();
            let body = || r.body().map(|b| b.to_string_lossy().into_owned());
            let mut copies = Vec::new();
            for map in [None, Some(mailmap.as_ref())] {
                for author in [true, false] {
                    let copy = if author {
                        r.author_with_mailmap(map)
                    } else {
                        r.committer_with_mailmap(map)
                    };
                    copies.push(copy.map(|s| rust_sig(s.as_ref())).map_err(|e| {
                        (
                            -1,
                            e.raw_class(),
                            e.message().to_string_lossy().into_owned(),
                        )
                    }));
                }
            }
            let got = (
                rust_sig(r.author()),
                rust_sig(r.committer()),
                body(),
                body(),
                copies,
            );
            assert_eq!(got, expected, "{hex}");
        }
        // SAFETY: the C side's objects, released once.
        unsafe {
            ffi::git_mailmap_free(c_mailmap);
            ffi::git_repository_free(c_repo);
        }
    }
}

/// Writing commits: [`create`], [`create_from_array`], [`create_buffer`],
/// [`create_ext`] and [`CommitRef::amend`] against `git_commit_create`,
/// `git_commit_create_v`, `git_commit_create_buffer`, `git_commit_create_ext` and
/// `git_commit_amend`.
///
/// Each side writes into a fresh bare repository of its own, one script of steps in
/// order: root commits and children, merges, every encoding and signature time (one
/// before the epoch and one past 2106, which libgit2 writes wrapped), a reference moved
/// from its first parent and refused from another, extra headers, amends of each part.
/// After every step the sides compare the outcome (the new id, the buffer, or the code,
/// class and message of the failure) and where their `HEAD` and `refs/heads/side`
/// point. Equal ids mean equal contents. Reference writes reach glibc's timezone state,
/// which another test's would race (TSan), so the script runs in an isolated child.
///
/// Documented divergence, outside these comparisons: the wrappers refuse a tree or a
/// parent of another repository (libgit2 asserts on the first, and drops the second
/// silently with every parent after it), and a signature libgit2 cannot write (one
/// without its name or email, or whose offset is `i32::MIN` minutes); the unit tests of
/// `commit.rs` hold the refusals.
///
/// [`create`]: libgit2::libgit2::object::commit::create
/// [`create_from_array`]: libgit2::libgit2::object::commit::create_from_array
/// [`create_buffer`]: libgit2::libgit2::object::commit::create_buffer
/// [`create_ext`]: libgit2::libgit2::object::commit::create_ext
/// [`CommitRef::amend`]: libgit2::libgit2::object::commit::CommitRef::amend
mod creating {
    use std::ffi::{CStr, CString, c_int};
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{SignatureView, Time};
    use libgit2::libgit2::object::commit::{
        Commit, CommitArc, CommitRef, CreateExtOptions, Header, create, create_buffer, create_ext,
        create_from_array,
    };
    use libgit2::libgit2::object::tree::Tree;
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryError};

    use super::init;

    const EMPTY_TREE: &CStr = c"4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    /// A signature: name, email, seconds, offset.
    type Who = (&'static CStr, &'static CStr, i64, c_int);

    const ALICE: Who = (c"Alice", c"alice@example.com", 1_700_000_000, 60);
    const BOB: Who = (c"Bob", c"bob@example.com", -5, -150);
    const LATE: Who = (c"Late", c"late@example.com", 1 << 33, 0);

    /// One step of the script; parents are indices of earlier successful commits.
    #[derive(Clone, Debug)]
    enum Step {
        Create(
            Option<&'static CStr>,
            Who,
            Who,
            Option<&'static CStr>,
            &'static CStr,
            Vec<usize>,
        ),
        Array(Option<&'static CStr>, Who, &'static CStr, Vec<usize>),
        Buffer(Who, Option<&'static CStr>, &'static CStr, Vec<usize>),
        Ext(
            Option<&'static CStr>,
            Option<&'static CStr>,
            &'static [(&'static CStr, &'static CStr)],
            &'static CStr,
            Vec<usize>,
        ),
        /// The commit, update_ref, author, committer, encoding, message, whether to
        /// pass the tree.
        Amend(
            usize,
            Option<&'static CStr>,
            Option<Who>,
            Option<Who>,
            Option<&'static CStr>,
            Option<&'static CStr>,
            bool,
        ),
    }

    fn script() -> Vec<Step> {
        use Step::*;
        let head = Some(c"HEAD");
        let side = Some(c"refs/heads/side");
        vec![
            Create(head, ALICE, BOB, None, c"root\n", vec![]),
            Create(head, ALICE, ALICE, None, c"second root\n", vec![]),
            Create(
                head,
                BOB,
                ALICE,
                Some(c"ISO-8859-1"),
                c"child\n\nbody\n",
                vec![0],
            ),
            Create(head, LATE, LATE, None, c"not from the tip\n", vec![0]),
            Array(head, BOB, c"merge\n", vec![1, 0]),
            Array(side, ALICE, c"side root\n", vec![]),
            Array(None, ALICE, c"octopus\n", vec![0, 1, 2, 3]),
            Buffer(ALICE, None, c"buffer\n", vec![]),
            Buffer(LATE, Some(c"UTF-16"), c"buffer\n", vec![2, 1]),
            Ext(
                side,
                Some(c"UTF-8"),
                &[(c"x-one", c"1"), (c"x-two", c"a\nb")],
                c"ext\n",
                vec![3],
            ),
            Ext(None, None, &[], c"plain ext\n", vec![]),
            Ext(head, None, &[], c"ext from a stale parent\n", vec![0]),
            Amend(3, None, None, None, None, None, false),
            Amend(
                3,
                None,
                Some(BOB),
                Some(LATE),
                Some(c"KOI8-R"),
                Some(c"amended\n"),
                true,
            ),
            Amend(0, head, None, None, None, Some(c"stale amend\n"), false),
            Amend(2, head, None, None, None, Some(c"tip amend\n"), false),
            Amend(5, side, None, None, None, None, false),
            Amend(
                5,
                Some(c"refs/heads/missing"),
                None,
                None,
                None,
                None,
                false,
            ),
            Create(
                Some(c"refs/heads/../bad"),
                ALICE,
                ALICE,
                None,
                c"bad name\n",
                vec![],
            ),
        ]
    }

    /// What a step gave: the id or buffer, or the failure.
    type Outcome = Result<String, (c_int, c_int, String)>;

    fn scratch(side: &str) -> (PathBuf, CString) {
        init();
        let dir = std::env::temp_dir().join(format!(
            "crustify-equiv-commit-create-{}-{side}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        (dir, path)
    }

    /// Where `HEAD` (through its branch) and `refs/heads/side` point, read back.
    fn refs(dir: &std::path::Path) -> (Option<String>, Option<String>) {
        let head = std::fs::read_to_string(dir.join("HEAD")).unwrap();
        let branch = head.trim().strip_prefix("ref: ").unwrap().to_owned();
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .ok()
                .map(|t| t.trim().to_owned())
        };
        (read(&branch), read("refs/heads/side"))
    }

    struct CSide {
        dir: PathBuf,
        repo: *mut ffi::git_repository,
        tree: *mut ffi::git_tree,
        commits: Vec<*mut ffi::git_commit>,
    }

    impl CSide {
        fn new() -> Self {
            let (dir, path) = scratch("c");
            let (mut repo, mut tree, mut id) =
                (ptr::null_mut(), ptr::null_mut(), ffi::git_oid::default());
            // SAFETY: writable slots, NUL-terminated strings, the live repository.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, path.as_ptr(), 1), 0);
                assert_eq!(
                    ffi::git_oid_from_string(&mut id, EMPTY_TREE.as_ptr(), ffi::GIT_OID_SHA1),
                    0
                );
                assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &id), 0);
            }
            Self {
                dir,
                repo,
                tree,
                commits: Vec::new(),
            }
        }

        fn failed(rc: c_int) -> Outcome {
            let (class, message) = super::c_last();
            Err((rc, class, message))
        }

        fn sig(who: Who) -> ffi::git_signature {
            ffi::git_signature {
                name: who.0.as_ptr().cast_mut(),
                email: who.1.as_ptr().cast_mut(),
                when: ffi::git_time {
                    time: who.2,
                    offset: who.3,
                    sign: if who.3 < 0 { b'-' } else { b'+' } as _,
                },
            }
        }

        fn hex(id: &ffi::git_oid) -> String {
            let mut out = [0u8; 41];
            // SAFETY: a live id; 41 writable bytes, NUL included.
            unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
            CStr::from_bytes_until_nul(&out)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        }

        /// Records a written commit, looked up for later steps.
        fn written(&mut self, rc: c_int, id: &ffi::git_oid) -> Outcome {
            if rc < 0 {
                return Self::failed(rc);
            }
            let mut commit = ptr::null_mut();
            // SAFETY: a writable slot, the live repository and the id just written.
            let rc = unsafe { ffi::git_commit_lookup(&mut commit, self.repo, id) };
            assert_eq!(rc, 0);
            self.commits.push(commit);
            Ok(Self::hex(id))
        }

        fn run(&mut self, step: &Step) -> Outcome {
            let s = |c: Option<&CStr>| c.map_or(ptr::null(), CStr::as_ptr);
            let mut id = ffi::git_oid::default();
            match step {
                Step::Create(update_ref, a, c, enc, msg, parents) => {
                    let ps: Vec<*const ffi::git_commit> = parents
                        .iter()
                        .map(|&i| self.commits[i].cast_const())
                        .collect();
                    let (a, c) = (Self::sig(*a), Self::sig(*c));
                    // SAFETY: a writable id, live objects of the repository, signatures
                    // over static strings, NULL or NUL-terminated strings.
                    let rc = unsafe {
                        ffi::git_commit_create(
                            &mut id,
                            self.repo,
                            s(*update_ref),
                            &a,
                            &c,
                            s(*enc),
                            msg.as_ptr(),
                            self.tree,
                            ps.len(),
                            ps.as_ptr().cast_mut(),
                        )
                    };
                    self.written(rc, &id)
                }
                Step::Array(update_ref, who, msg, parents) => {
                    let p = |k: usize| self.commits[parents[k]].cast_const();
                    let w = Self::sig(*who);
                    let (r, t, u, m) = (self.repo, self.tree, s(*update_ref), msg.as_ptr());
                    // SAFETY: as above, the parents passed variadically, as many as the
                    // count says.
                    let rc = unsafe {
                        match parents.len() {
                            0 => ffi::git_commit_create_v(
                                &mut id,
                                r,
                                u,
                                &w,
                                &w,
                                ptr::null(),
                                m,
                                t,
                                0,
                            ),
                            2 => ffi::git_commit_create_v(
                                &mut id,
                                r,
                                u,
                                &w,
                                &w,
                                ptr::null(),
                                m,
                                t,
                                2,
                                p(0),
                                p(1),
                            ),
                            4 => ffi::git_commit_create_v(
                                &mut id,
                                r,
                                u,
                                &w,
                                &w,
                                ptr::null(),
                                m,
                                t,
                                4,
                                p(0),
                                p(1),
                                p(2),
                                p(3),
                            ),
                            n => unreachable!("{n} parents"),
                        }
                    };
                    self.written(rc, &id)
                }
                Step::Buffer(who, enc, msg, parents) => {
                    let ps: Vec<*const ffi::git_commit> = parents
                        .iter()
                        .map(|&i| self.commits[i].cast_const())
                        .collect();
                    let w = Self::sig(*who);
                    let mut buf = ffi::git_buf::default();
                    // SAFETY: a zeroed buffer libgit2 fills; as in `Create`.
                    let rc = unsafe {
                        ffi::git_commit_create_buffer(
                            &mut buf,
                            self.repo,
                            &w,
                            &w,
                            s(*enc),
                            msg.as_ptr(),
                            self.tree,
                            ps.len(),
                            ps.as_ptr().cast_mut(),
                        )
                    };
                    if rc < 0 {
                        return Self::failed(rc);
                    }
                    // SAFETY: libgit2 left `size` initialised bytes at `ptr`; released once.
                    let text = unsafe {
                        let t = String::from_utf8(
                            std::slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.size).to_vec(),
                        )
                        .unwrap();
                        ffi::git_buf_dispose(&mut buf);
                        t
                    };
                    Ok(text)
                }
                Step::Ext(update_ref, enc, headers, msg, parents) => {
                    let ps: Vec<*mut ffi::git_commit> =
                        parents.iter().map(|&i| self.commits[i]).collect();
                    let hs: Vec<ffi::git_commit_header> = headers
                        .iter()
                        .map(|(f, v)| ffi::git_commit_header {
                            field: f.as_ptr(),
                            value: v.as_ptr(),
                        })
                        .collect();
                    let opts = ffi::git_commit_create_ext_options {
                        version: 1,
                        update_ref: s(*update_ref),
                        message_encoding: s(*enc),
                        extra_headers: if hs.is_empty() {
                            ptr::null()
                        } else {
                            hs.as_ptr()
                        },
                        extra_headers_len: hs.len(),
                        ..Default::default()
                    };
                    let w = Self::sig(ALICE);
                    // SAFETY: as in `Create`; options over live strings and headers.
                    let rc = unsafe {
                        ffi::git_commit_create_ext(
                            &mut id,
                            self.repo,
                            &w,
                            &w,
                            msg.as_ptr(),
                            self.tree,
                            ps.len(),
                            ps.as_ptr(),
                            &opts,
                        )
                    };
                    self.written(rc, &id)
                }
                Step::Amend(i, update_ref, a, c, enc, msg, tree) => {
                    let (a, c) = (a.map(Self::sig), c.map(Self::sig));
                    let sp = |x: &Option<ffi::git_signature>| {
                        x.as_ref().map_or(ptr::null(), |x| x as *const _)
                    };
                    // SAFETY: as in `Create`; NULL parts are the commit's own.
                    let rc = unsafe {
                        ffi::git_commit_amend(
                            &mut id,
                            self.commits[*i],
                            s(*update_ref),
                            sp(&a),
                            sp(&c),
                            s(*enc),
                            s(*msg),
                            if *tree { self.tree } else { ptr::null() },
                        )
                    };
                    self.written(rc, &id)
                }
            }
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            // SAFETY: the side's objects, each released once, the repository last.
            unsafe {
                for &c in &self.commits {
                    ffi::git_commit_free(c);
                }
                ffi::git_tree_free(self.tree);
                ffi::git_repository_free(self.repo);
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn rust_failed(e: RepositoryError) -> Outcome {
        Err((
            e.raw_code(),
            e.raw_class(),
            e.message().to_string_lossy().into_owned(),
        ))
    }

    fn view(who: Who) -> SignatureView<'static> {
        SignatureView::new(who.0, who.1, Time::new(who.2, who.3))
    }

    /// The commits at `idx`, lent.
    fn lent<'a, 'r>(commits: &'a [CommitArc<'r>], idx: &[usize]) -> Vec<CommitRef<'a, 'r>> {
        idx.iter().map(|&i| commits[i].as_ref()).collect()
    }

    /// The Rust side's step, through the public API only.
    fn rust_run<'r>(
        repo: &'r RepositoryBox,
        commits: &mut Vec<CommitArc<'r>>,
        step: &Step,
    ) -> Outcome {
        let r = repo.as_ref();
        let tree = Tree::lookup(
            r,
            Oid::from_string(EMPTY_TREE, OidType::Sha1)
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let result = match step {
            Step::Create(update_ref, a, c, enc, msg, parents) => {
                let (a, c) = (view(*a), view(*c));
                create(
                    r,
                    *update_ref,
                    a.as_ref(),
                    c.as_ref(),
                    *enc,
                    msg,
                    tree.as_ref(),
                    &lent(commits, parents),
                )
            }
            Step::Array(update_ref, who, msg, parents) => {
                let w = view(*who);
                let ps = lent(commits, parents);
                let (w, t) = (w.as_ref(), tree.as_ref());
                match ps.len() {
                    0 => create_from_array(r, *update_ref, w, w, None, msg, t, []),
                    2 => create_from_array(r, *update_ref, w, w, None, msg, t, [ps[0], ps[1]]),
                    4 => create_from_array(
                        r,
                        *update_ref,
                        w,
                        w,
                        None,
                        msg,
                        t,
                        [ps[0], ps[1], ps[2], ps[3]],
                    ),
                    n => unreachable!("{n} parents"),
                }
            }
            Step::Buffer(who, enc, msg, parents) => {
                let w = view(*who);
                return match create_buffer(
                    r,
                    w.as_ref(),
                    w.as_ref(),
                    *enc,
                    msg,
                    tree.as_ref(),
                    &lent(commits, parents),
                ) {
                    Ok(buf) => Ok(buf.as_ref().to_str().unwrap().to_owned()),
                    Err(e) => rust_failed(e),
                };
            }
            Step::Ext(update_ref, enc, headers, msg, parents) => {
                let hs: Vec<Header<'static>> =
                    headers.iter().map(|(f, v)| Header::new(f, v)).collect();
                let mut options = CreateExtOptions::new();
                let mut m = options.as_mut();
                m.set_update_ref(*update_ref);
                m.set_message_encoding(*enc);
                m.set_extra_headers(&hs);
                let w = view(ALICE);
                create_ext(
                    r,
                    w.as_ref(),
                    w.as_ref(),
                    msg,
                    tree.as_ref(),
                    &lent(commits, parents),
                    Some(options.as_mut()),
                )
            }
            Step::Amend(i, update_ref, a, c, enc, msg, with_tree) => {
                let (a, c) = (a.map(view), c.map(view));
                commits[*i].as_ref().amend(
                    *update_ref,
                    a.as_ref().map(SignatureView::as_ref),
                    c.as_ref().map(SignatureView::as_ref),
                    *enc,
                    *msg,
                    with_tree.then(|| tree.as_ref()),
                )
            }
        };
        match result {
            Ok(id) => {
                commits.push(Commit::lookup(r, id.as_ref()).unwrap());
                Ok(id.as_ref().to_hex_string().unwrap())
            }
            Err(e) => rust_failed(e),
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "BorrowSanitizer reports git_commit_create_v's later variadic parents as freed"
    )]
    fn creation_scripts_match() {
        super::isolated(
            concat!(module_path!(), "::creation_scripts_match"),
            creation_scripts_match_body,
        );
    }

    fn creation_scripts_match_body() {
        let mut c = CSide::new();
        let (dir, path) = scratch("rust");
        let rust = Repository::init(&path, true).unwrap();
        let mut commits = Vec::new();
        let mut failures = 0;
        for (n, step) in script().iter().enumerate() {
            let expected = c.run(step);
            let got = rust_run(&rust, &mut commits, step);
            assert_eq!(got, expected, "step {n}: {step:?}");
            assert_eq!(refs(&dir), refs(&c.dir), "step {n}: {step:?}");
            failures += usize::from(expected.is_err());
        }
        // The second root, the stale-parent create, ext and amend, the amend of a
        // missing reference, and the bad name failed alike.
        assert_eq!(failures, 6);
        drop(commits);
        drop(rust);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Turns every loose fan-out directory `objects/xx` that the repository at `dir`
    /// lacks into a regular file: writing an object there then fails as the directory
    /// cannot be made, even as root.
    fn block_loose_writes(dir: &std::path::Path) {
        for byte in 0..=u8::MAX {
            let fan_out = dir.join(format!("objects/{byte:02x}"));
            if !fan_out.exists() {
                std::fs::write(fan_out, "").unwrap();
            }
        }
    }

    /// `outcome` with the repository's directory, which a failure's message names,
    /// masked.
    fn masked(outcome: Outcome, dir: &std::path::Path) -> Outcome {
        let dir = dir.to_str().unwrap();
        outcome.map_err(|(rc, class, message)| (rc, class, message.replace(dir, "<repo>")))
    }

    /// Every creation routine, on a repository whose object database cannot be written
    /// once its root commit is: `git_commit__create_internal` builds the contents and
    /// fails at `git_odb_write`, before moving any reference. Each side compares the
    /// code, class and message (its directory masked), and where the references point,
    /// after each step. The amends move no reference: `git_commit_amend` leaks the one it
    /// resolved for `update_ref` when the write fails.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[cfg_attr(
        bsan,
        ignore = "BorrowSanitizer reports git_commit_create_v's later variadic parents as freed"
    )]
    fn unwritable_object_databases_fail_alike() {
        super::isolated(
            concat!(module_path!(), "::unwritable_object_databases_fail_alike"),
            unwritable_object_databases_fail_alike_body,
        );
    }

    fn unwritable_object_databases_fail_alike_body() {
        use Step::*;
        let head = Some(c"HEAD");
        let root = Create(head, ALICE, BOB, None, c"root\n", vec![]);
        let steps = [
            Create(head, ALICE, BOB, None, c"child\n", vec![0]),
            Create(
                None,
                BOB,
                ALICE,
                Some(c"ISO-8859-1"),
                c"another root\n",
                vec![],
            ),
            Array(head, BOB, c"array child\n", vec![0, 0]),
            Buffer(ALICE, None, c"buffer\n", vec![0]),
            Ext(head, Some(c"UTF-8"), &[(c"x-one", c"1")], c"ext\n", vec![0]),
            Amend(0, None, None, None, None, Some(c"amended\n"), false),
            Amend(0, None, Some(BOB), Some(LATE), None, None, true),
        ];
        let mut c = CSide::new();
        let (dir, path) = scratch("rust");
        let rust = Repository::init(&path, true).unwrap();
        let mut commits = Vec::new();
        assert_eq!(rust_run(&rust, &mut commits, &root), c.run(&root));
        block_loose_writes(&dir);
        block_loose_writes(&c.dir);
        let mut failures = 0;
        for (n, step) in steps.iter().enumerate() {
            let expected = masked(c.run(step), &c.dir);
            let got = masked(rust_run(&rust, &mut commits, step), &dir);
            assert_eq!(got, expected, "step {n}: {step:?}");
            assert_eq!(refs(&dir), refs(&c.dir), "step {n}: {step:?}");
            failures += usize::from(expected.is_err());
        }
        // Every write failed (the buffer is not written); none landed in the root's
        // fan-out directory.
        assert_eq!(failures, steps.len() - 1);
        drop(commits);
        drop(rust);
        let _ = std::fs::remove_dir_all(dir);
    }
}
