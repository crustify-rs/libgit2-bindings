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

use std::ffi::CStr;
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
        let back = Commit::try_from_object(object).unwrap();
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
            let converted = Commit::try_from_object(object);
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
    struct Signer {
        rc: Cell<c_int>,
        calls: Cell<usize>,
    }

    /// Adds a fixed `gpgsig` header through the lent builder, counts the call and
    /// returns the payload's code.
    unsafe extern "C" fn sign(
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
    fn rust_sign(signer: &Signer, mut builder: CommitBuilderMut<'_>) -> Result<(), StreamError> {
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

/// Equivalence of `create_with_signature` and `extract_signature` with
/// `git_commit_create_with_signature` and `git_commit_extract_signature`.
///
/// Each side has its own bare repository holding the empty tree, and runs one script:
/// commits written unsigned and signed (default and custom fields, multi-line
/// signatures, a parent), contents that do not parse, signed contents without a blank
/// line, a missing tree; then every written commit's signature extracted under each
/// field, from commits with no, one and two signature headers, from a blob and from a
/// missing id. After every step the ids, the two buffers, or the codes, error classes
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
            ] {
                let c_out = c_extract(&c, id, field);
                found += usize::from(c_out.is_ok());
                assert_eq!(rust_extract(&rust, id, field), c_out, "{id:x?} {field:?}");
            }
        }
        assert!(found >= 6, "{found}");
    }
}
