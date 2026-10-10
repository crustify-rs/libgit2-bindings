//! UB tests for the unsafe API of `git2/commit.h`.
//!
//! `CommitArray::from_raw`, called within its contract on every shape of
//! `git_commitarray` libgit2 leaves behind or C code builds, edge cases included, and
//! then driven through the safe API: read, emptied, cleared, handed back with
//! `into_raw` and adopted again, and released before its repository, as the contract
//! requires (ASan: double free, invalid free, use after free, overflow; LSan: leak).
//!
//! `CreateExtOptionsMut::set_sign_callback`, called within its contract: callbacks and
//! payloads that stay valid while the options are passed to `git_commit_create_ext`,
//! replaced, cleared, and called once per commit, adding the signature header through
//! the builder libgit2 lends, refusing, or passing; the payload is a Rust value the
//! callback writes, read back after the call (ASan, BSan: a retained or stale pointer).

use std::ffi::CStr;
use std::mem::size_of;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::object::commit::{CommitArray, CommitArrayRef, CommitArrayVal, CommitRef};

const HEAD: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
const MERGED: [&str; 2] = [
    "3a34580a35add43a4cf361e8e9a30060a905c876",
    "e2809157a7766f272e4cfe26e61ef2678a5357ff",
];

/// Opens the fixture repository `name` under `tests/resources`.
fn open(name: &str) -> *mut ffi::git_repository {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock. The
    // reference dies with the process.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
    let path = std::ffi::CString::new(format!(
        "{}/../../../tests/resources/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut repo = ptr::null_mut();
    // SAFETY: a live slot and a NUL-terminated path; no ceiling directories.
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

fn free_repo(repo: *mut ffi::git_repository) {
    // SAFETY: the tests free each repository they open once.
    unsafe { ffi::git_repository_free(repo) };
}

fn hex(commit: CommitRef<'_, '_>) -> String {
    let commit = commit.to_commit();
    let mut out = [0u8; 41];
    // SAFETY: a commit its owner keeps alive; 41 writable bytes.
    unsafe {
        ffi::git_oid_tostr(
            out.as_mut_ptr().cast(),
            out.len(),
            ffi::git_commit_id(commit.as_c_ptr()),
        )
    };
    CStr::from_bytes_until_nul(&out)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

fn ids(array: CommitArrayRef<'_, '_>) -> Vec<String> {
    array.iter().map(hex).collect()
}

/// `git_repository_commit_parents` into a zeroed struct.
fn parents(repo: *mut ffi::git_repository) -> ffi::git_commitarray {
    let mut raw = ffi::git_commitarray::default();
    // SAFETY: a zeroed array and an open repository.
    let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
    assert_eq!(rc, 0);
    raw
}

/// A new reference to the commit `hex` in `repo`.
fn lookup(repo: *mut ffi::git_repository, hex: &CStr) -> *mut ffi::git_commit {
    let mut id = ffi::git_oid::default();
    // SAFETY: a writable oid and a NUL-terminated hex string.
    let rc = unsafe { ffi::git_oid_from_string(&mut id, hex.as_ptr(), ffi::GIT_OID_SHA1) };
    assert_eq!(rc, 0);
    let mut commit = ptr::null_mut();
    // SAFETY: a live slot, an open repository and a live oid.
    assert_eq!(unsafe { ffi::git_commit_lookup(&mut commit, repo, &id) }, 0);
    commit
}

/// A buffer of libgit2's allocator for `slots` commit pointers, the first `commits`
/// filled from `commits`, the rest NULL: as C code builds an array (with spare
/// capacity when `slots` is larger).
fn c_built(commits: &[*mut ffi::git_commit], slots: usize) -> ffi::git_commitarray {
    assert!(slots >= commits.len() && slots > 0);
    // SAFETY: the routine ignores `backend`; the runtime is initialised.
    let buf = unsafe {
        ffi::git_odb_backend_data_alloc(ptr::null_mut(), slots * size_of::<*mut ffi::git_commit>())
    }
    .cast::<*mut ffi::git_commit>();
    assert!(!buf.is_null());
    for i in 0..slots {
        let commit = commits.get(i).copied().unwrap_or(ptr::null_mut());
        // SAFETY: `buf` holds `slots` pointer slots, suitably aligned (libgit2's
        // allocator is malloc).
        unsafe { buf.add(i).write(commit) };
    }
    ffi::git_commitarray {
        commits: buf,
        count: commits.len(),
    }
}

/// Every libgit2 result, adopted: read in bounds, cleared, filled again through
/// `into_raw`, adopted again, emptied with `take` and dropped before the repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_what_libgit2_fills() {
    for (name, expected) in [
        ("testrepo.git", &[HEAD][..]),
        ("mergedrepo/.gitted", &MERGED[..]),
        ("empty_bare.git", &[][..]),
    ] {
        let repo = open(name);
        // SAFETY: what libgit2 left, given up here: `{NULL, 0}`, or a buffer of
        // exactly `count` references to commits (these repositories' `HEAD`s and
        // `MERGE_HEAD`s are commits) from its allocator, released before `repo`.
        let mut array = unsafe { CommitArray::from_raw(parents(repo)) };
        assert_eq!(ids(array.as_ref()), expected, "{name}");
        assert!(array.as_ref().get(expected.len()).is_none());
        array.as_mut().clear();
        assert!(array.as_ref().is_empty());

        let mut raw = array.into_inner().into_raw();
        assert!(raw.commits.is_null() && raw.count == 0);
        // SAFETY: `raw` is empty, so nothing is overwritten unreleased.
        let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
        assert_eq!(rc, 0);
        // SAFETY: as above: libgit2 refilled it.
        let mut again: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(raw) };
        assert_eq!(ids(again.as_ref()), expected, "{name}: refilled");
        let taken = again.as_mut().take();
        assert_eq!(taken.len(), expected.len());
        assert!(again.as_ref().is_empty());
        drop(again);
        drop(taken);
        free_repo(repo);
    }
}

/// Arrays C code builds over libgit2's allocator, with spare capacity, with a buffer
/// but no commits, and handed back to C's dispose through `into_raw`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn adopts_what_c_code_builds() {
    let repo = open("mergedrepo/.gitted");
    let commits = [
        lookup(repo, c"3a34580a35add43a4cf361e8e9a30060a905c876"),
        lookup(repo, c"e2809157a7766f272e4cfe26e61ef2678a5357ff"),
        lookup(repo, c"3a34580a35add43a4cf361e8e9a30060a905c876"),
    ];
    // SAFETY: a libgit2 buffer of 5 slots owning the 3 commit references looked up
    // above, given up here, released before `repo`; the 2 spare slots are never read
    // or released.
    let array = unsafe { CommitArray::from_raw(c_built(&commits, 5)) };
    assert_eq!(ids(array.as_ref()), [MERGED[0], MERGED[1], MERGED[0]]);
    assert!(array.as_ref().get(3).is_none());
    drop(array);

    // The same shape, emptied with `take`: the spare slots go with the buffer.
    let commits = [lookup(repo, c"e2809157a7766f272e4cfe26e61ef2678a5357ff")];
    // SAFETY: as above, one reference in 4 slots.
    let mut array = unsafe { CommitArray::from_raw(c_built(&commits, 4)) };
    let taken = array.as_mut().take();
    assert_eq!(hex(taken[0].as_ref()), MERGED[1]);
    drop((array, taken));

    // SAFETY: a libgit2 buffer owning no commits, given up here.
    let mut empty_buffer: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(c_built(&[], 2)) };
    assert!(empty_buffer.as_ref().is_empty());
    empty_buffer.as_mut().clear();
    // SAFETY: a second such buffer, released by the drop.
    drop(unsafe { CommitArray::<'_>::from_raw(c_built(&[], 1)) });
    // SAFETY: a third, emptied with `take`, which frees the buffer.
    let mut taken: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(c_built(&[], 3)) };
    assert!(taken.as_mut().take().is_empty());

    let one = [lookup(repo, c"e2809157a7766f272e4cfe26e61ef2678a5357ff")];
    // SAFETY: a libgit2 buffer owning one commit reference, given up here; it leaves
    // through `into_raw` before `repo` is freed.
    let adopted: CommitArrayVal<'_> = unsafe { CommitArray::from_raw(c_built(&one, 1)) };
    let mut back = adopted.into_inner().into_raw();
    assert_eq!(back.count, 1);
    // SAFETY: `back` owns its commit and buffer again, released once by C.
    unsafe { ffi::git_commitarray_dispose(&mut back) };
    assert!(back.commits.is_null() && back.count == 0);
    free_repo(repo);
}

/// A NULL buffer carries no requirement: whatever the count, the array is empty and
/// released without libgit2, so this runs under Miri.
#[test]
fn adopts_a_null_buffer_with_any_count() {
    for count in [0, 1, 7, usize::MAX] {
        // SAFETY: `commits` is NULL.
        let mut array: CommitArrayVal<'static> = unsafe {
            CommitArray::from_raw(ffi::git_commitarray {
                commits: ptr::null(),
                count,
            })
        };
        assert_eq!(array.as_ref().len(), 0);
        assert!(array.as_ref().get(0).is_none());
        assert_eq!(format!("{:?}", array.as_ref()), "[]");
        if count == 7 {
            assert!(array.as_mut().take().is_empty());
        } else if count % 2 == 1 {
            array.as_mut().clear();
            let raw = array.into_inner().into_raw();
            assert_eq!((raw.commits, raw.count), (ptr::null(), 0));
        }
    }
}

/// `CreateExtOptionsMut::set_sign_callback`, within its contract: every callback is a
/// function sound to call with libgit2's arguments, and every payload a live
/// [`Signer`] the test owns, outliving each call the options are passed to.
mod sign_callback {
    use std::cell::{Cell, RefCell};
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::{CreateExtOptions, Header};

    use super::free_repo;

    /// What a signing callback is told to return, and what it saw. Shared between the
    /// test and the callback, which reach it through shared borrows only: the payload
    /// pointer is derived from one, so the test's reads and writes never invalidate it.
    struct Signer {
        rc: Cell<c_int>,
        calls: Cell<usize>,
        content: RefCell<Vec<u8>>,
        signature: CString,
    }

    impl Signer {
        fn new(rc: c_int, signature: &str) -> Box<Self> {
            Box::new(Self {
                rc: Cell::new(rc),
                calls: Cell::new(0),
                content: RefCell::new(Vec::new()),
                signature: CString::new(signature).unwrap(),
            })
        }

        /// The payload for the callback: a pointer from a shared borrow.
        fn payload(&self) -> *mut c_void {
            ptr::from_ref(self).cast_mut().cast()
        }
    }

    /// Adds `payload`'s signature as `gpgsig` through the lent builder, then returns
    /// the payload's code.
    unsafe extern "C" fn sign(
        builder: *mut ffi::git_commitbuilder,
        repo: *mut ffi::git_repository,
        content: *const c_char,
        payload: *mut c_void,
    ) -> c_int {
        assert!(!repo.is_null());
        // SAFETY: the payload is the test's live `Signer`, shared-borrowed.
        let signer = unsafe { &*payload.cast::<Signer>() };
        signer.calls.set(signer.calls.get() + 1);
        // SAFETY: libgit2 passes the commit's contents, NUL-terminated, live for the
        // call; they are copied.
        *signer.content.borrow_mut() = unsafe { CStr::from_ptr(content) }.to_bytes().to_vec();
        // SAFETY: the builder libgit2 lends for the call, and NUL-terminated strings.
        let rc = unsafe {
            ffi::git_commitbuilder_add_header(
                builder,
                c"gpgsig".as_ptr(),
                signer.signature.as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        signer.rc.get()
    }

    /// Counts its call in `payload`, a live `Cell<usize>`, and signs nothing.
    unsafe extern "C" fn count(
        _: *mut ffi::git_commitbuilder,
        _: *mut ffi::git_repository,
        _: *const c_char,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: the payload is the test's live counter, shared-borrowed.
        let counter = unsafe { &*payload.cast::<Cell<usize>>() };
        counter.set(counter.get() + 1);
        0
    }

    /// A repository with the empty tree and a signature, freed on drop.
    struct Repo {
        repo: *mut ffi::git_repository,
        tree: *mut ffi::git_tree,
        sig: *mut ffi::git_signature,
        dir: std::path::PathBuf,
    }

    impl Repo {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "crustify-commit-sign-ub-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
            // The reference dies with the process.
            assert!(unsafe { ffi::git_libgit2_init() } > 0);
            let path = CString::new(dir.to_str().unwrap()).unwrap();
            let (mut repo, mut builder, mut id, mut tree, mut sig) = (
                ptr::null_mut(),
                ptr::null_mut(),
                ffi::git_oid::default(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            // SAFETY: writable slots, NUL-terminated strings; each object is freed on
            // drop.
            unsafe {
                assert_eq!(ffi::git_repository_init(&mut repo, path.as_ptr(), 1), 0);
                assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
                assert_eq!(ffi::git_treebuilder_write(&mut id, builder), 0);
                ffi::git_treebuilder_free(builder);
                assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &id), 0);
                assert_eq!(
                    ffi::git_signature_new(&mut sig, c"A".as_ptr(), c"a@b".as_ptr(), 1, 0),
                    0
                );
            }
            Self {
                repo,
                tree,
                sig,
                dir,
            }
        }

        /// `git_commit_create_ext` with `options`; the code and the raw commit.
        fn create(&self, options: &CreateExtOptions<'_>) -> (c_int, String) {
            let mut id = ffi::git_oid::default();
            // SAFETY: the repository's live objects; the options are a live value
            // (`CreateExtOptions` is transparent over the C struct) whose callback and
            // payload the caller keeps valid for the call.
            let rc = unsafe {
                ffi::git_commit_create_ext(
                    &mut id,
                    self.repo,
                    self.sig,
                    self.sig,
                    c"signed\n".as_ptr(),
                    self.tree,
                    0,
                    ptr::null(),
                    ptr::from_ref(options).cast(),
                )
            };
            if rc != 0 {
                return (rc, String::new());
            }
            let (mut odb, mut obj) = (ptr::null_mut(), ptr::null_mut());
            // SAFETY: the commit just written is read back and copied.
            unsafe {
                assert_eq!(ffi::git_repository_odb(&mut odb, self.repo), 0);
                assert_eq!(ffi::git_odb_read(&mut obj, odb, &id), 0);
                let data = std::slice::from_raw_parts(
                    ffi::git_odb_object_data(obj).cast::<u8>(),
                    ffi::git_odb_object_size(obj),
                );
                let text = String::from_utf8(data.to_vec()).unwrap();
                ffi::git_odb_object_free(obj);
                ffi::git_odb_free(odb);
                (rc, text)
            }
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            // SAFETY: each object is released once, the repository last.
            unsafe {
                ffi::git_signature_free(self.sig);
                ffi::git_tree_free(self.tree);
            }
            free_repo(self.repo);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// One call per commit, the header added through the lent builder; a positive
    /// code writes the commit, a negative one (`GIT_PASSTHROUGH` too) fails the call;
    /// the payload outlives every call and is read back after each.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn the_callback_signs_each_commit_once() {
        let repo = Repo::new("signs");
        let headers = [Header::new(c"x-extra", c"before the signature")];
        let signer = Signer::new(0, "-----BEGIN-----\nsig\n-----END-----");
        let mut options = CreateExtOptions::new();
        options.as_mut().set_extra_headers(&headers);
        // SAFETY: `sign` is sound with libgit2's arguments and a live `Signer`;
        // `signer` outlives every call below, and is only ever shared-borrowed.
        unsafe {
            options
                .as_mut()
                .set_sign_callback(Some(sign), signer.payload())
        };
        let (rc, text) = repo.create(&options);
        assert_eq!((rc, signer.calls.get()), (0, 1));
        assert!(
            text.contains(
                "\nx-extra before the signature\ngpgsig -----BEGIN-----\n sig\n -----END-----\n"
            ),
            "{text}"
        );
        let unsigned = String::from_utf8(signer.content.borrow().clone()).unwrap();
        assert!(!unsigned.contains("gpgsig") && unsigned.ends_with("\nsigned\n"));

        for (code, written) in [(1, true), (-30, false), (-1, false)] {
            signer.rc.set(code);
            let (rc, text) = repo.create(&options);
            assert_eq!(rc, if written { 0 } else { code }, "{code}");
            assert_eq!(text.contains("gpgsig"), written);
        }
        assert_eq!(signer.calls.get(), 4);
        drop(signer);
    }

    /// A replaced callback is not called again; a cleared one calls nothing; the
    /// payload of each is a different live value, freed after the options are.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn replaced_and_cleared_callbacks() {
        let repo = Repo::new("replaced");
        let signer = Signer::new(0, "sig");
        let counter = Box::new(Cell::new(0usize));
        let mut options = CreateExtOptions::new();
        // SAFETY: as in `the_callback_signs_each_commit_once`.
        unsafe {
            options
                .as_mut()
                .set_sign_callback(Some(sign), signer.payload())
        };
        // SAFETY: `count` is sound with libgit2's arguments and a live `Cell<usize>`,
        // shared-borrowed, which outlives the calls below.
        unsafe {
            options
                .as_mut()
                .set_sign_callback(Some(count), ptr::from_ref(&*counter).cast_mut().cast())
        };
        assert_eq!(repo.create(&options).0, 0);
        assert_eq!(repo.create(&options).0, 0);
        assert_eq!((counter.get(), signer.calls.get()), (2, 0));
        // SAFETY: nothing is stored to call.
        unsafe { options.as_mut().set_sign_callback(None, ptr::null_mut()) };
        drop(counter);
        let (rc, text) = repo.create(&options);
        assert_eq!(rc, 0);
        assert!(!text.contains("gpgsig"));
        assert_eq!(options.as_ref().sign_callback().1, ptr::null_mut());
        drop(signer);
    }
}
