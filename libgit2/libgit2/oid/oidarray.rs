//! Safe wrappers over `include/git2/oidarray.h`.
//!
//! An [`OidArray`] is a `git_oidarray`: a run of `count` object ids, `ids`, in
//! libgit2's allocator. libgit2 never allocates the struct itself: callers hold it by
//! value, libgit2 fills it (`git_merge_bases`, `git_merge_bases_many`) or reads one a
//! custom transport filled (`shallow_roots` in `git2/sys/transport.h`), and
//! `git_oidarray_dispose` frees `ids` with `git__free`, leaving the struct to its
//! holder. Rust holds it the same way, as an [`OidArrayVal`], which disposes it on drop.
//!
//! `ids` is the struct's own allocation, so it moves in and out as an
//! [`OdbDataVec<Oid>`]: the counted run that `git_odb_backend_data_alloc` /
//! `git_odb_backend_data_free` settle, which are `git__malloc` / `git__free`. A buffer
//! libgit2 filled (a `git_array` grown with `git__reallocarray`, or a `git__malloc`'d
//! copy) is released by the same `git__free`, whatever its spare capacity.
//!
//! Every handle upholds one invariant, which the safe setters keep and a crate-internal
//! `from_ptr` over a C-owned struct must vouch for: `ids` is NULL with `count == 0`, or
//! a live buffer of at least `count` ids from libgit2's allocator that the struct owns.
//! libgit2's own producers leave `ids` NULL for an empty result; an empty run built in
//! Rust is NULL too.
//!
//! There is no public `OidArray` value outside an owner: one holding ids would leak
//! them when dropped, so the constructors return an [`OidArrayVal`], and `OidArray` is
//! not `CZeroable`. [`CVal::into_inner`] still surrenders the bare value, as
//! `mem::forget` surrenders a `Vec`.

use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CSlice, CSliceMut, CVal, define_ctype, impl_cdispose};

use crate::ffi;
use crate::libgit2::oid::oid::Oid;
use crate::libgit2::raw::{OdbDataArrayFree, OdbDataVec};

define_ctype!(
    /// An array of object ids (`git_oidarray`): `count` ids in one buffer of
    /// libgit2's allocator, which the struct owns.
    ///
    /// Held by value inside an [`OidArrayVal`], which [`new`](Self::new),
    /// [`from_ids`](Self::from_ids) and [`from_slice`](Self::from_slice) build; read
    /// and write it through the owner's [`as_ref`](CVal::as_ref) /
    /// [`as_mut`](CVal::as_mut) handles. Not `Clone`: it owns its buffer, so a copy is
    /// [`OidArrayRef::to_oid_array`], which allocates.
    ///
    /// Wraps: git_oidarray
    OidArray, OidArrayRef, OidArrayMut, ffi::git_oidarray
);

// SAFETY: an `OidArray` is a pointer to a buffer of plain `git_oid` bytes it owns
// exclusively, and its length. The buffer is libgit2-allocator storage, which
// `git__free` releases on any thread (as `OdbDataVec<Oid>`, which is `Send`), and no
// libgit2 routine keeps thread-local state about it; moving the struct moves sole
// ownership of it.
unsafe impl Send for OidArray {}
// SAFETY: `OidArrayMut` is an exclusive borrow of such a value, as `&mut OidArray`.
unsafe impl Send for OidArrayMut<'_> {}

// SAFETY: shared access only reads `ids`, `count` and the ids they address: every
// write goes through `OidArrayMut`, which excludes other handles, and no libgit2
// routine takes a `const git_oidarray *` to write through.
unsafe impl Sync for OidArray {}
// SAFETY: as above; `OidArrayRef` is a shared borrow, as `&OidArray`.
unsafe impl Send for OidArrayRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for OidArrayRef<'_> {}
// SAFETY: `&OidArrayMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for OidArrayMut<'_> {}

/// Releases an array's ids through `git_oidarray_dispose`, which hands `ids` to
/// `git__free` and leaves the struct, Rust's storage, alone.
///
/// Wraps: git_oidarray_dispose
#[derive(Clone, Copy, Debug, Default)]
pub struct OidArrayDispose;

// `git_oidarray_dispose` frees `ids` exactly once and never the struct. It accepts
// every state the safe API reaches: the empty array (`git__free(NULL)`, a no-op even
// before `git_libgit2_init`, when the allocator is `failalloc`), and a buffer
// `set_ids` stored from an `OdbDataVec`, i.e. from libgit2's allocator; `truncate` only
// lowers `count`, which the free ignores, and `take_ids` leaves the empty array.
impl_cdispose!(OidArrayDispose, OidArray, ffi::git_oidarray_dispose);

/// An owned [`OidArray`], held inline; its ids are released by `git_oidarray_dispose`
/// on drop.
pub type OidArrayVal = CVal<OidArray, OidArrayDispose>;

impl OidArray {
    /// An empty array: no ids, nothing allocated, as `git_oidarray array = {0}`. The
    /// storage libgit2 fills a result into.
    pub fn new() -> OidArrayVal {
        CVal::new(Self(
            ffi::git_oidarray {
                ids: ptr::null_mut(),
                count: 0,
            },
            PhantomData,
        ))
    }

    /// An array owning `ids`, as libgit2 hands one out: the buffer becomes the array's,
    /// released by `git_oidarray_dispose`.
    pub fn from_ids(ids: OdbDataVec<Oid>) -> OidArrayVal {
        let mut array = Self::new();
        array.as_mut().set_ids(ids);
        array
    }

    /// An array holding copies of `ids`, in a fresh buffer of libgit2's allocator, as a
    /// custom transport's `shallow_roots` hands libgit2 one; empty and unallocated for
    /// no ids.
    ///
    /// `None` when the allocation fails, or libgit2 cannot be initialised.
    pub fn from_slice(ids: &[Oid]) -> Option<OidArrayVal> {
        OdbDataVec::from_slice(ids).map(Self::from_ids)
    }
}

impl<'a> OidArrayRef<'a> {
    /// The number of ids.
    ///
    /// Field: git_oidarray.count
    pub fn len(&self) -> usize {
        // SAFETY: a read through the shared handle's pointer to a live `git_oidarray`;
        // any `size_t` is a valid `usize`.
        unsafe { addr_of!((*self.as_ptr()).count).read() }
    }

    /// Whether the array holds no ids.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The ids, borrowed in place as a run of [`len`](Self::len) handles for as long as
    /// the array is.
    ///
    /// Field: git_oidarray.ids
    pub fn ids(&self) -> CSlice<'a, Oid> {
        let len = self.len();
        // SAFETY: a read through the shared handle's pointer to a live `git_oidarray`.
        let ids = unsafe { addr_of!((*self.as_ptr()).ids).read() };
        match NonNull::new(ids.cast::<Oid>()) {
            // SAFETY: per the handles' invariant a non-NULL `ids` addresses at least
            // `count` initialised `git_oid`s (`Oid` is transparent over one), owned by
            // the array, so they live as long as it does, past `'a`; the run only reads
            // them, as the handle does.
            Some(ids) => unsafe { CSlice::from_raw_parts(ids, len) },
            // NULL is the empty array, whose `count` is 0; a run may not start at NULL.
            // SAFETY: zero elements at an aligned dangling pointer.
            None => unsafe { CSlice::from_raw_parts(NonNull::dangling(), 0) },
        }
    }

    /// A copy of the array, its ids in a fresh buffer of libgit2's allocator; libgit2
    /// has no copy routine. Empty and unallocated for an empty array.
    ///
    /// `None` when the allocation fails, or libgit2 cannot be initialised.
    ///
    /// The owner itself has no `Clone`: a byte copy would dispose the ids twice.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::oid::oidarray::{OidArray, OidArrayVal};
    ///
    /// let array = OidArray::new();
    /// let copy: OidArrayVal = array.clone();
    /// ```
    pub fn to_oid_array(&self) -> Option<OidArrayVal> {
        let src = self.ids();
        let mut copy = OdbDataArrayFree::new_uninit::<Oid>(src.len())?;
        // SAFETY: `src` is `src.len()` initialised ids, readable while the array is
        // borrowed; `copy` is a fresh, distinct buffer of as many, so the two do not
        // overlap (both are aligned and dangling when empty). `git_oid` is plain bytes.
        unsafe {
            ptr::copy_nonoverlapping(
                src.as_ptr().cast::<Oid>(),
                copy.as_mut_slice().as_mut_ptr().cast::<Oid>(),
                src.len(),
            );
        }
        // SAFETY: every element was written by the copy above (there are none when
        // `src` is empty).
        Some(OidArray::from_ids(unsafe { copy.assume_init() }))
    }
}

impl fmt::Debug for OidArrayRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.ids().iter()).finish()
    }
}

impl fmt::Debug for OidArrayMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl OidArrayMut<'_> {
    /// The ids, borrowed exclusively in place as a run of handles, for editing them, as
    /// libgit2 reverses a result in place.
    ///
    /// Field: git_oidarray.ids
    pub fn ids_mut(&mut self) -> CSliceMut<'_, Oid> {
        let len = self.as_ref().len();
        // SAFETY: a read through the exclusive handle's pointer to a live
        // `git_oidarray`.
        let ids = unsafe { addr_of!((*self.as_mut_ptr()).ids).read() };
        match NonNull::new(ids.cast::<Oid>()) {
            // SAFETY: per the handles' invariant a non-NULL `ids` addresses at least
            // `count` initialised ids owned by the array; this handle is borrowed
            // mutably for the result's lifetime, so nothing else reaches them meanwhile.
            Some(ids) => unsafe { CSliceMut::from_raw_parts(ids, len) },
            // SAFETY: zero elements at an aligned dangling pointer, as in `ids`.
            None => unsafe { CSliceMut::from_raw_parts(NonNull::dangling(), 0) },
        }
    }

    /// Replaces the ids, and with them the count, releasing the ones held before.
    ///
    /// Field: git_oidarray.ids
    /// Field: git_oidarray.count
    pub fn set_ids(&mut self, ids: OdbDataVec<Oid>) {
        drop(self.take_ids());
        let (ids, count) = ids.into_raw_parts();
        let array = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to a live
        // `git_oidarray`, whose previous buffer `take_ids` released. `ids` is NULL with
        // `count == 0`, or `count` ids in libgit2's allocator whose ownership the array
        // takes over, which keeps the handles' invariant.
        unsafe {
            addr_of_mut!((*array).ids).write(ids.cast());
            addr_of_mut!((*array).count).write(count);
        }
    }

    /// Takes the ids out, leaving the array empty: what `git_oidarray_dispose` would
    /// release, owned.
    ///
    /// Field: git_oidarray.ids
    pub fn take_ids(&mut self) -> OdbDataVec<Oid> {
        let array = self.as_mut_ptr();
        // SAFETY: reads, then writes the empty array, through the exclusive handle's
        // pointer to a live `git_oidarray`.
        let (ids, count) = unsafe {
            let ids = addr_of!((*array).ids).read();
            let count = addr_of!((*array).count).read();
            addr_of_mut!((*array).ids).write(ptr::null_mut());
            addr_of_mut!((*array).count).write(0);
            (ids, count)
        };
        // SAFETY: per the handles' invariant `ids` is NULL with `count == 0`, or at
        // least `count` initialised ids in libgit2's allocator, owned by the array and
        // now handed over whole, since the array no longer refers to them.
        // `OdbDataArrayFree` frees with `git__free`, which ignores the length, so spare
        // capacity beyond `count` is released with the buffer.
        let ids = unsafe { OdbDataVec::from_raw_parts_or_empty(ids.cast::<Oid>(), count) };
        ids.unwrap_or_default()
    }

    /// Shortens the array to its first `len` ids; nothing changes when it holds no more
    /// than that. The buffer is kept, and released whole on dispose.
    ///
    /// Field: git_oidarray.count
    pub fn truncate(&mut self, len: usize) {
        if len < self.as_ref().len() {
            // SAFETY: a write through the exclusive handle's pointer to a live
            // `git_oidarray`; a smaller count keeps the buffer at least `count` long.
            unsafe { addr_of_mut!((*self.as_mut_ptr()).count).write(len) }
        }
    }
}

// An `OidArray` is a `git_oidarray`, so C reads and writes a value Rust holds in place,
// and the owner adds nothing to it.
const _: () = assert!(
    size_of::<OidArray>() == size_of::<ffi::git_oidarray>()
        && align_of::<OidArray>() == align_of::<ffi::git_oidarray>()
        && size_of::<OidArrayVal>() == size_of::<ffi::git_oidarray>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::oid::oid::{OidRef, OidType};
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use crate::libgit2::util::sys::errors::runtime_pinned;
    use core::ffi::{CStr, c_int, c_void};
    use core::mem::size_of;
    use ffibox::{CDispose, CZeroable};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
    fn isolated(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = Command::new(std::env::current_exe().unwrap())
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

    static REFUSING: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// libc's allocator, refusing everything while `REFUSING` is set.
    struct Refusing;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Refusing {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if REFUSING.load(SeqCst) {
                return None;
            }
            // SAFETY: plain libc allocation.
            NonNull::new(unsafe { malloc(len) }.cast())
        }
        unsafe fn realloc(
            p: Option<NonNull<u8>>,
            len: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            if REFUSING.load(SeqCst) {
                return None;
            }
            let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            NonNull::new(unsafe { realloc(p, len) }.cast())
        }
        unsafe fn free(p: NonNull<u8>) {
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// A SHA1 oid whose id bytes are `seed`, `seed + 1`, ..., built in Rust alone.
    fn oid(seed: u8) -> Oid {
        let mut oid = Oid::zeroed();
        oid.as_mut().set_kind(OidType::Sha1);
        oid.as_mut()
            .set_id(core::array::from_fn(|i| seed.wrapping_add(i as u8)));
        oid
    }

    fn bytes(oid: OidRef<'_>) -> (u8, [u8; 32]) {
        (oid.raw_kind(), oid.id())
    }

    /// The ids of `array`, copied out as bytes.
    fn contents(array: OidArrayRef<'_>) -> Vec<(u8, [u8; 32])> {
        array.ids().iter().map(bytes).collect()
    }

    fn expected(seeds: &[u8]) -> Vec<(u8, [u8; 32])> {
        seeds.iter().map(|&s| bytes(oid(s).as_ref())).collect()
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<OidArray>(), size_of::<ffi::git_oidarray>());
        assert_eq!(size_of::<OidArrayVal>(), size_of::<ffi::git_oidarray>());
        assert_eq!(
            size_of::<OidArrayRef<'_>>(),
            size_of::<*const ffi::git_oidarray>()
        );
        assert_eq!(
            size_of::<OidArrayMut<'_>>(),
            size_of::<*const ffi::git_oidarray>()
        );
        assert_eq!(size_of::<OidArrayDispose>(), 0);
    }

    /// The empty array's accessors over a bare value, without libgit2, so Miri runs
    /// them; nothing here allocates or disposes.
    #[test]
    fn empty_array_accessors_in_rust_storage() {
        let mut bare = OidArray::new().into_inner();
        let mut m = bare.as_mut();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().is_empty());
        assert!(m.as_ref().ids().is_empty());
        assert!(m.as_ref().ids().get(0).is_none());
        assert!(m.ids_mut().is_empty());
        assert!(m.ids_mut().get_mut(0).is_none());
        m.truncate(0);
        m.truncate(usize::MAX);
        assert!(m.take_ids().is_empty());
        m.set_ids(OdbDataVec::empty());
        assert_eq!(m.as_ref().len(), 0);
        assert_eq!(format!("{m:?}"), "[]");
        assert_eq!(format!("{:?}", bare.as_ref()), "[]");
        // SAFETY: a read of the value Rust holds: still the empty array.
        let raw = unsafe { addr_of!(bare.0).read() };
        assert!(raw.ids.is_null() && raw.count == 0);
    }

    /// The handles' projections, runs and moves over a buffer Rust allocated, so Miri
    /// checks them; every buffer leaves through `into_raw_parts`, so nothing reaches
    /// libgit2's free.
    #[test]
    fn handles_over_rust_storage() {
        let mut storage: Vec<Oid> = (0..4).map(|i| oid(10 * i)).collect();
        let base = storage.as_mut_ptr();
        let mut raw = ffi::git_oidarray {
            ids: base.cast(),
            count: 4,
        };
        // SAFETY: `raw` is live for the handle's use and addresses 4 ids, which the
        // test keeps alive; no other handle reaches it meanwhile. The buffer is never
        // released through the handle: every take below hands it back raw.
        let mut m = unsafe { OidArrayMut::from_ptr(&mut raw) }.unwrap();
        assert_eq!(contents(m.as_ref()), expected(&[0, 10, 20, 30]));
        {
            let mut run = m.ids_mut();
            run.get_mut(3).unwrap().set_raw_kind(9);
            assert!(run.get_mut(4).is_none());
            let (mut a, mut b) = run.split_at_mut(2).unwrap();
            a.get_mut(1).unwrap().set_id([1; 32]);
            b.get_mut(0).unwrap().set_id([2; 32]);
        }
        let r = m.as_ref();
        assert_eq!(r.ids().get(1).unwrap().id(), [1; 32]);
        assert_eq!(r.ids().get(2).unwrap().id(), [2; 32]);
        assert_eq!(r.ids().get(3).unwrap().raw_kind(), 9);
        m.truncate(3);
        assert_eq!(m.as_ref().ids().len(), 3);

        let (ids, count) = m.take_ids().into_raw_parts();
        assert_eq!((ids, count), (base, 3));
        assert!(m.as_ref().is_empty());
        // SAFETY: `ids` is the test's own buffer of 4 ids; the vector is handed back
        // raw below and never dropped, so `OdbDataArrayFree` never frees it.
        let adopted = unsafe { OdbDataVec::from_raw_parts(ids, 4) }.unwrap();
        m.set_ids(adopted);
        assert_eq!(m.as_ref().len(), 4);
        assert_eq!(m.as_ref().ids().get(0).unwrap().id(), oid(0).as_ref().id());
        let (ids, count) = m.take_ids().into_raw_parts();
        assert_eq!((ids, count), (base, 4));
        assert_eq!((raw.ids, raw.count), (ptr::null_mut(), 0));
        assert_eq!(storage[1].as_ref().id(), [1; 32]);
    }

    /// Copies, edits and replacements each release every buffer once (LSan, in the
    /// asan variant, flags a leak, ASan a double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owned_arrays_copy_edit_and_release() {
        let src = [oid(1), oid(40), oid(80)];
        let mut array = OidArray::from_slice(&src).unwrap();
        assert_eq!(array.as_ref().len(), 3);
        assert!(!array.as_ref().is_empty());
        assert_eq!(contents(array.as_ref()), expected(&[1, 40, 80]));

        let copy = array.as_ref().to_oid_array().unwrap();
        assert_ne!(
            copy.as_ref().ids().as_ptr(),
            array.as_ref().ids().as_ptr(),
            "a copy has a buffer of its own"
        );
        array
            .as_mut()
            .ids_mut()
            .get_mut(0)
            .unwrap()
            .copy_from(oid(9).as_ref());
        assert_eq!(contents(array.as_ref()), expected(&[9, 40, 80]));
        assert_eq!(contents(copy.as_ref()), expected(&[1, 40, 80]));

        array.as_mut().truncate(5);
        assert_eq!(array.as_ref().len(), 3);
        array.as_mut().truncate(1);
        assert_eq!(contents(array.as_ref()), expected(&[9]));
        // A truncated buffer copies only what is left, and releases whole.
        let short = array.as_ref().to_oid_array().unwrap();
        assert_eq!(contents(short.as_ref()), expected(&[9]));

        array
            .as_mut()
            .set_ids(OdbDataVec::from_slice(&[oid(2), oid(3)]).unwrap());
        assert_eq!(contents(array.as_ref()), expected(&[2, 3]));
        let taken = array.as_mut().take_ids();
        assert_eq!(taken.len(), 2);
        assert!(array.as_ref().is_empty());
        assert!(array.as_ref().ids().is_empty());
        let moved = OidArray::from_ids(taken);
        assert_eq!(contents(moved.as_ref()), expected(&[2, 3]));
        assert_eq!(
            format!("{:?}", moved.as_ref()),
            format!("[{:?}, {:?}]", oid(2), oid(3))
        );
        assert_eq!(format!("{:?}", array.as_mut()), "[]");

        let empty = OidArray::from_slice(&[]).unwrap();
        assert!(empty.as_ref().is_empty());
        let empty_copy = empty.as_ref().to_oid_array().unwrap();
        assert!(empty_copy.as_ref().is_empty());
    }

    /// The dispose policy, driven by hand on a bare value: its ids are freed once
    /// (LSan flags a leak otherwise).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn dispose_policy_releases_the_ids() {
        let mut bare = OidArray::from_slice(&[oid(5)]).unwrap().into_inner();
        assert_eq!(bare.as_ref().len(), 1);
        // SAFETY: `bare` is a live array Rust owns, disposed once and not used after.
        unsafe { OidArrayDispose.c_dispose(NonNull::from(&mut bare)) };
    }

    /// Handles over a C-owned `git_oidarray`, as libgit2 hands a callback one: the
    /// setters' buffers are what `git_oidarray_dispose` releases, and C's buffers what
    /// they release (ASan flags an allocator mismatch, LSan a leak).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn handles_over_a_c_owned_array() {
        assert!(runtime_pinned());
        let len = 2 * size_of::<ffi::git_oid>();
        // SAFETY: the routine ignores its backend; the runtime is pinned.
        let ids = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), len) };
        assert!(!ids.is_null());
        let ids = ids.cast::<ffi::git_oid>();
        for (i, seed) in [7u8, 8].into_iter().enumerate() {
            // SAFETY: `ids` holds two `git_oid`s; `Oid` is transparent over one.
            unsafe { ids.add(i).cast::<Oid>().write(oid(seed)) };
        }
        let mut raw = ffi::git_oidarray { ids, count: 2 };

        // SAFETY: `raw` is live for the handle's use and owns `ids` (two ids from
        // libgit2's allocator); no other handle reaches it meanwhile.
        let mut m = unsafe { OidArrayMut::from_ptr(&mut raw) }.unwrap();
        assert_eq!(contents(m.as_ref()), expected(&[7, 8]));
        m.ids_mut().get_mut(1).unwrap().copy_from(oid(70).as_ref());
        // Replacing frees C's buffer through `git__free`.
        m.set_ids(OdbDataVec::from_slice(&[oid(1), oid(2), oid(3)]).unwrap());
        assert_eq!(raw.count, 3);
        // SAFETY: `raw` is live and only read here.
        let r = unsafe { OidArrayRef::from_ptr(&mut raw) }.unwrap();
        assert_eq!(contents(r), expected(&[1, 2, 3]));
        // C releases the buffer Rust stored.
        // SAFETY: `raw` owns `ids`, from libgit2's allocator, and is not used again.
        unsafe { ffi::git_oidarray_dispose(&mut raw) };

        // An array that breaks the invariant with a NULL `ids` and a count reads as
        // empty, and is reset without a free, rather than dereferencing NULL.
        let mut broken = ffi::git_oidarray {
            ids: ptr::null_mut(),
            count: 3,
        };
        // SAFETY: `broken` is live for the handle's use; nothing else reaches it.
        let mut m = unsafe { OidArrayMut::from_ptr(&mut broken) }.unwrap();
        assert_eq!(m.as_ref().len(), 3);
        assert!(m.as_ref().ids().is_empty());
        assert!(m.ids_mut().is_empty());
        assert!(m.take_ids().is_empty());
        assert_eq!((broken.ids, broken.count), (ptr::null_mut(), 0));
    }

    /// A failed allocation is `None` from both copying constructors, and leaves the
    /// source untouched.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copies_out_of_memory_are_none() {
        isolated(
            concat!(module_path!(), "::copies_out_of_memory_are_none"),
            || {
                let allocator = Allocator::new::<Refusing>();
                // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`,
                // with every member set; libgit2 copies it before anything is
                // allocated.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                        allocator.as_ref().as_ptr(),
                    )
                };
                assert_eq!(rc, 0);
                assert!(runtime_pinned());
                let array = OidArray::from_slice(&[oid(1), oid(2)]).unwrap();
                REFUSING.store(true, SeqCst);
                let copied = OidArray::from_slice(&[oid(3)]).is_none();
                let cloned = array.as_ref().to_oid_array().is_none();
                // Nothing to allocate for an empty array.
                let empty = OidArray::from_slice(&[]).is_some();
                REFUSING.store(false, SeqCst);
                assert!(copied && cloned && empty);
                assert_eq!(contents(array.as_ref()), expected(&[1, 2]));
            },
        );
    }

    /// Without libgit2's runtime nothing can be allocated, but the empty array needs
    /// none, and disposing it is `failalloc`'s no-op free.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copies_without_runtime_fail_cleanly() {
        isolated(
            concat!(module_path!(), "::copies_without_runtime_fail_cleanly"),
            || {
                REFUSING.store(true, SeqCst);
                let allocator = Allocator::new::<Refusing>();
                // SAFETY: as in `copies_out_of_memory_are_none`.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                        allocator.as_ref().as_ptr(),
                    )
                };
                assert_eq!(rc, 0);
                let empty = OidArray::new();
                assert!(OidArray::from_slice(&[oid(1)]).is_none());
                assert!(empty.as_ref().to_oid_array().is_some());
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                drop(empty);
            },
        );
    }
}
