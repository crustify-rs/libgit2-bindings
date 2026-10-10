//! Equivalence of `git2/commit.h`'s [`CommitArray`] with the `git_commitarray` C code
//! holds, has libgit2 fill and releases with `git_commitarray_dispose`.
//!
//! Each array is produced twice, independently, one copy per side, by
//! `git_repository_commit_parents` on its own open repository: `testrepo.git` (one
//! parent), `mergedrepo` (a merge in progress: `HEAD` and `MERGE_HEAD`) and
//! `empty_bare.git` (an unborn `HEAD`: `{NULL, 0}`). The Rust side adopts its copy as
//! libgit2 hands it over, as the `git_repository_commit_parents` wrapper will. Both
//! sides then read the array (count, and each commit's id), release it (C:
//! `git_commitarray_dispose`; Rust: [`CommitArrayMut::clear`]), have libgit2 fill it
//! again, read it after freeing the repository, and release it for good (C: dispose;
//! Rust: drop). After every step both record the struct's fields as the public API
//! exposes them and the results are compared.
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
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef, CommitArrayVal};

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

/// The Rust side's view, through the public API only.
fn rust_record(array: CommitArrayRef<'_>) -> Record {
    (array.len(), array.commit_ptrs().elems().map(hex).collect())
}

/// `git_repository_commit_parents` into `raw`, which must hold nothing.
fn c_fill(raw: &mut ffi::git_commitarray, repo: *mut ffi::git_repository) -> i32 {
    // SAFETY: `raw` is writable and empty, so nothing is overwritten unreleased;
    // `repo` is open.
    unsafe { ffi::git_repository_commit_parents(raw, repo) }
}

/// The same fill, adopted by the Rust side.
fn rust_fill(repo: *mut ffi::git_repository) -> (i32, CommitArrayVal) {
    let mut raw = ffi::git_commitarray::default();
    let rc = c_fill(&mut raw, repo);
    // SAFETY: `raw` is what libgit2 left: `{NULL, 0}`, or a buffer of commit references
    // from its allocator, given up here.
    (rc, unsafe { CommitArray::from_raw(raw) })
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn empty_arrays_match() {
    // C's dispose of the empty array still reaches `git__free(NULL)`, which reads the
    // allocator a concurrent first `git_libgit2_init` writes: initialise first.
    init();
    let mut raw = ffi::git_commitarray::default();
    let mut array = CommitArray::new();
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
        assert_eq!(
            raw.commits.is_null(),
            array.as_ref().commit_ptrs().is_empty()
        );

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
        let array = unsafe { CommitArray::from_raw(cleared) };
        assert_eq!(c_rc, rust_rc, "{name:?}: refill");
        assert_eq!(c_record(&raw), filled);
        assert_eq!(rust_record(array.as_ref()), filled, "{name:?}: refill");

        // The commits outlive their repositories.
        for repo in [c_repo, rust_repo] {
            // SAFETY: each repository is open and freed once.
            unsafe { ffi::git_repository_free(repo) };
        }
        assert_eq!(
            c_record(&raw),
            rust_record(array.as_ref()),
            "{name:?}: orphaned"
        );

        // SAFETY: `raw` owns its refill, released once.
        unsafe { ffi::git_commitarray_dispose(&mut raw) };
        drop(array);
        assert_eq!(c_record(&raw), (0, Vec::new()));
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
