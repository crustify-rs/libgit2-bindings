//! Equivalence of `git2/refs.h`'s reference owners with the C lifecycle they wrap.
//!
//! [`ReferenceBox`]'s `clone` / `try_clone` must copy as `git_reference_dup` does.
//! Each probe reads one reference twice from the same repository, once for C and once
//! for Rust (adopted into a box, as the lookup wrapper will), copies each side twice
//! (a copy of the original, then a copy of the copy), drops the originals and then the
//! first copies, and compares the two sides after every step: the result of the copy,
//! and what C reads out of each copy — name, type, target id, peeled id, symbolic
//! target, `git_reference_cmp` against its counterpart, and `git_reference_owner`. The
//! probes cover a symbolic reference (`HEAD`), a loose direct one, and a packed,
//! peeled tag.
//!
//! [`UnattachedReference::direct`] must build what `git_reference__alloc` builds, and
//! [`UnattachedReference::symbolic`] what `git_reference__alloc_symbolic` does, for
//! names from empty to long and of any bytes, ids of every type byte, with and
//! without a peeled id: both sides are read back through the same C getters and
//! compared with `git_reference_cmp`. Then each side is released through its own path
//! (`git_reference_free` on the C pointer, the box's drop on the Rust one), which
//! ASan's leak check holds to exactly once.
//!
//! Documented divergence, outside these comparisons: `git_reference_dup` of a
//! reference from `git_reference__alloc` dereferences its NULL reference database and
//! crashes. An [`UnattachedReference`] has no copy, so the Rust side cannot reach it.

use std::ffi::{CStr, CString, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::{fs, ptr};

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::refs::refs::{ReferenceBox, UnattachedReference};
use libgit2::libgit2::repository::repository::{InitFlag, InitOptions, Repository};

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

/// A fresh bare repository with a loose branch and a packed, peeled tag; freed and
/// removed on drop.
struct Repo {
    path: PathBuf,
    repo: *mut ffi::git_repository,
}

impl Repo {
    fn new(tag: &str) -> Self {
        init();
        let path =
            std::env::temp_dir().join(format!("crustify-refs-equiv-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let mut opts = InitOptions::new();
        opts.as_mut().set_flags(InitFlag::Bare | InitFlag::Mkdir);
        // The raw repository both sides' calls use; `Drop` frees it.
        let repo = Repository::init_ext(&c_path, opts.as_mut())
            .unwrap()
            .into_c();
        fs::write(
            path.join("packed-refs"),
            format!(
                "# pack-refs with: peeled fully-peeled sorted \n{} refs/tags/v1\n^{}\n",
                "11".repeat(20),
                "22".repeat(20)
            ),
        )
        .unwrap();
        fs::write(
            path.join("refs/heads/loose"),
            format!("{}\n", "33".repeat(20)),
        )
        .unwrap();
        Self { path, repo }
    }

    /// `git_reference_lookup(name)`: a fresh reference the caller frees.
    fn lookup(&self, name: &CStr) -> *mut ffi::git_reference {
        let mut out = ptr::null_mut();
        // SAFETY: a live slot, the live repository and a NUL-terminated name, read for
        // the call.
        let rc = unsafe { ffi::git_reference_lookup(&mut out, self.repo, name.as_ptr()) };
        assert_eq!(rc, 0, "lookup {name:?}: {:?}", c_last());
        out
    }

    /// The same lookup, adopted for the Rust side: no safe constructor of an attached
    /// reference exists yet.
    fn lookup_owned(&self, name: &CStr) -> ReferenceBox<'_> {
        // SAFETY: a fresh attached reference the caller owns, released by
        // `git_reference_free`; it borrows `self`, which frees the repository last.
        unsafe { ReferenceBox::from_c(self.lookup(name)) }.unwrap()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        // SAFETY: the repository `new` opened, freed once; every reference read from
        // it is gone.
        unsafe { ffi::git_repository_free(self.repo) };
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// What C reads out of a reference.
#[derive(Clone, Debug, PartialEq, Eq)]
struct View {
    name: CString,
    kind: ffi::git_reference_t,
    target: Option<(u8, [u8; 32])>,
    peel: Option<(u8, [u8; 32])>,
    symbolic: Option<CString>,
}

fn id(oid: *const ffi::git_oid) -> Option<(u8, [u8; 32])> {
    // SAFETY: NULL, or an id inside a live reference, copied out at once.
    (!oid.is_null()).then(|| unsafe { ((*oid).type_, (*oid).id) })
}

/// C's view of the live reference at `p`.
fn view(p: *const ffi::git_reference) -> View {
    // SAFETY: `p` is a live reference, read for the calls only; every string and id it
    // hands back lives as long as it and is copied at once.
    unsafe {
        let symbolic = ffi::git_reference_symbolic_target(p);
        View {
            name: CStr::from_ptr(ffi::git_reference_name(p)).to_owned(),
            kind: ffi::git_reference_type(p),
            target: id(ffi::git_reference_target(p)),
            peel: id(ffi::git_reference_target_peel(p)),
            symbolic: (!symbolic.is_null()).then(|| CStr::from_ptr(symbolic).to_owned()),
        }
    }
}

/// `git_reference_cmp`.
fn cmp(a: *const ffi::git_reference, b: *const ffi::git_reference) -> c_int {
    // SAFETY: two live references, read for the call only.
    unsafe { ffi::git_reference_cmp(a, b) }
}

/// `git_reference_owner` of an attached reference.
fn owner(p: *const ffi::git_reference) -> *mut ffi::git_repository {
    // SAFETY: a live attached reference, whose repository is open.
    unsafe { ffi::git_reference_owner(p) }
}

/// `git_reference_dup`: its result, and the copy on success.
fn c_dup(p: *mut ffi::git_reference) -> (c_int, *mut ffi::git_reference) {
    let mut out = ptr::null_mut();
    // SAFETY: a live slot and a live attached reference, read for the call.
    let rc = unsafe { ffi::git_reference_dup(&mut out, p) };
    (rc, out)
}

/// Frees a C reference once.
fn c_free(p: *mut ffi::git_reference) {
    // SAFETY: a live reference the caller owns, freed once.
    unsafe { ffi::git_reference_free(p) }
}

/// Asserts the two sides read alike, compare equal, and reach the same repository.
fn same(c: *mut ffi::git_reference, rust: &ReferenceBox<'_>, repo: &Repo, step: &str) {
    let r = rust.as_c_ptr();
    assert_eq!(view(c), view(r), "{step}");
    assert_eq!(cmp(c, r), 0, "{step}");
    assert_eq!(owner(c), repo.repo, "{step}");
    assert_eq!(owner(r), repo.repo, "{step}");
}

/// Copies, copies of copies and the survivors of dropped originals match C's.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_match_git_reference_dup() {
    let repo = Repo::new("dup");
    for name in [c"HEAD", c"refs/heads/loose", c"refs/tags/v1"] {
        let c_original = repo.lookup(name);
        let rust_original = repo.lookup_owned(name);
        same(c_original, &rust_original, &repo, "lookup");

        let (rc, c_copy) = c_dup(c_original);
        let rust_copy = rust_original.try_clone();
        assert_eq!(rc == 0, rust_copy.is_some(), "{name:?}: first copy");
        let rust_copy = rust_copy.unwrap();
        assert_ne!(c_copy, c_original);
        assert_ne!(rust_copy.as_c_ptr(), rust_original.as_c_ptr());
        same(c_copy, &rust_copy, &repo, "first copy");

        let (rc, c_copy2) = c_dup(c_copy);
        assert_eq!(rc, 0);
        let rust_copy2 = rust_copy.clone();
        same(c_copy2, &rust_copy2, &repo, "copy of the copy");

        c_free(c_original);
        drop(rust_original);
        same(c_copy, &rust_copy, &repo, "original dropped");
        same(c_copy2, &rust_copy2, &repo, "original dropped");

        c_free(c_copy);
        drop(rust_copy);
        same(c_copy2, &rust_copy2, &repo, "first copy dropped");
        c_free(c_copy2);
    }
}

/// The ids every direct probe points at: each named type, an unknown type byte, and
/// none at all.
fn ids() -> Vec<Oid> {
    let mut unknown = Oid::zeroed();
    unknown.as_mut().set_raw_kind(7);
    unknown.as_mut().set_id([0x99; 32]);
    vec![
        Oid::from_raw(&[0x44; 20], OidType::Sha1).unwrap(),
        Oid::from_raw(&[0x55; 32], OidType::Sha256).unwrap(),
        Oid::zero(OidType::Sha1),
        unknown,
        Oid::zeroed(),
    ]
}

/// An independent C copy of `oid`, for the C side.
fn c_oid(oid: &Oid) -> ffi::git_oid {
    ffi::git_oid {
        type_: oid.as_ref().raw_kind(),
        id: oid.as_ref().id(),
    }
}

/// Names from empty to long, of any bytes.
fn names() -> Vec<CString> {
    vec![
        CString::new("").unwrap(),
        CString::new("HEAD").unwrap(),
        CString::new("refs/heads/main").unwrap(),
        CString::new(vec![0xFF, b'/', 0x80]).unwrap(),
        CString::new(vec![b'n'; 2048]).unwrap(),
    ]
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn direct_matches_git_reference_alloc() {
    init();
    let ids = ids();
    for name in names() {
        for target in &ids {
            for peel in [None].into_iter().chain(ids.iter().map(Some)) {
                let c_target = c_oid(target);
                let c_peel = peel.map(c_oid);
                // SAFETY: a NUL-terminated name and live ids, copied; a fresh reference
                // the caller owns, or NULL.
                let c = unsafe {
                    ffi::git_reference__alloc(
                        name.as_ptr(),
                        &c_target,
                        c_peel.as_ref().map_or(ptr::null(), ptr::from_ref),
                    )
                };
                assert!(!c.is_null());
                let rust =
                    UnattachedReference::direct(&name, target.as_ref(), peel.map(Oid::as_ref))
                        .unwrap();
                let r = rust.as_c_ptr();
                assert_eq!(view(c), view(r));
                assert_eq!(cmp(c, r), 0);
                c_free(c);
            }
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn symbolic_matches_git_reference_alloc_symbolic() {
    init();
    for name in names() {
        for target in names() {
            // SAFETY: two NUL-terminated names, copied; a fresh reference the caller
            // owns, or NULL.
            let c = unsafe { ffi::git_reference__alloc_symbolic(name.as_ptr(), target.as_ptr()) };
            assert!(!c.is_null());
            let rust = UnattachedReference::symbolic(&name, &target).unwrap();
            let r = rust.as_c_ptr();
            assert_eq!(view(c), view(r));
            assert_eq!(cmp(c, r), 0);
            c_free(c);
        }
    }
}
