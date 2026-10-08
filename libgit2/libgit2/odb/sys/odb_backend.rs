//! `git2/sys/odb_backend.h`: the interface custom object-database backends implement.
//!
//! The header also publishes libgit2.so's only exported raw byte allocator,
//! `git_odb_backend_data_alloc` / `git_odb_backend_data_free`: a backend's `read`
//! callbacks return buffers from it, which libgit2 later releases with its own
//! `git__free`. Both routines ignore their `backend` argument (`GIT_UNUSED`) and
//! forward to `git__malloc` / `git__free`, so the policies below are stateless and
//! pass NULL for it.
//!
//! [`OdbDataFree`] owns one object ([`OdbDataBox`]), [`OdbDataArrayFree`] a counted
//! run of elements ([`OdbDataVec`]). Neither zeroes on its own: `git__malloc` is a
//! plain `malloc`, so zeroed constructors clear the storage in Rust, as `git__calloc`
//! does in C.
//!
//! The first allocation pins libgit2's runtime with a `git_libgit2_init` reference
//! of its own, held for the rest of the process (`runtime_pinned`). C callers
//! get NULL before initialisation instead, through a path that corrupts foreign
//! thread-local state; the strategies deliberately diverge from it.
//!
//! Storage handed out by a custom `GIT_OPT_SET_ALLOCATOR` allocator is released
//! through whichever allocator is current when the owner drops, so the allocator must
//! not be replaced while an owner is live — the contract of that option, not of these
//! policies.

use core::mem::{MaybeUninit, align_of, size_of};
use core::ptr::{self, NonNull};
use std::sync::OnceLock;

use ffibox::{
    CAlloc, CAllocZeroed, CBox, CCell, CDrop, CPlainElem, CVec, CZeroable, impl_clenclone,
    impl_clendrop,
};

use crate::ffi;

/// The alignment `git_odb_backend_data_alloc` storage is guaranteed to have.
///
/// It is `git__allocator.gmalloc` storage, in which libgit2 itself places structs
/// holding pointers, `size_t`s and 64-bit integers, so any allocator libgit2 runs on
/// aligns that far. The default `stdalloc` is glibc's `malloc`, which aligns to 16.
const DATA_ALIGN: usize = {
    let (word, wide) = (align_of::<usize>(), align_of::<u64>());
    if word > wide { word } else { wide }
};

/// Holds one `git_libgit2_init` reference for the rest of the process; `false` when
/// libgit2 cannot be initialised.
///
/// Until the first initialisation, and after the last shutdown, libgit2's allocator
/// fails every request and `git__malloc` reports it through `git_error_set_oom`,
/// which looks up thread-local state under a TLS key that does not exist: it writes
/// into whatever block key 0 holds. A reference that is never released keeps the key
/// live across balanced `git_libgit2_init` / `git_libgit2_shutdown` pairs.
///
/// Raw seam: replace with the safe `git_libgit2_init` wrapper (`core/global.rs`).
fn runtime_pinned() -> bool {
    static PINNED: OnceLock<bool> = OnceLock::new();
    *PINNED.get_or_init(|| {
        // SAFETY: no arguments; libgit2 counts initialisations under its own lock, so
        // concurrent callers are serialised. A negative result is a failed
        // initialisation, after which nothing is allocated.
        let count = unsafe { ffi::git_libgit2_init() };
        count >= 0
    })
}

/// `git_odb_backend_data_alloc` for at least `byte_len` bytes; `None` on failure,
/// or when libgit2 cannot be initialised.
///
/// A zero-byte request asks for one byte instead: `malloc(0)` may return NULL, which
/// would read as failure.
fn data_alloc(byte_len: usize) -> Option<NonNull<u8>> {
    if !runtime_pinned() {
        return None;
    }
    // SAFETY: the routine never reads `backend` (`GIT_UNUSED`), so NULL is accepted;
    // it returns fresh `git__malloc` storage, or NULL on failure, for any `len`. The
    // runtime is pinned, so a failure is reported on live thread-local state.
    let data = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), byte_len.max(1)) };
    NonNull::new(data.cast())
}

/// `git_odb_backend_data_free`.
///
/// # Safety
///
/// `data` must be a live allocation from libgit2's current allocator
/// (`git_odb_backend_data_alloc`), which the caller owns and does not use again.
unsafe fn data_free(data: *mut u8) {
    // SAFETY: the routine never reads `backend`; `data` is a live `git__malloc`
    // allocation the caller gives up, per this function's contract.
    unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), data.cast()) }
}

/// One object in libgit2's allocator, released through `git_odb_backend_data_free`.
///
/// A storage policy: its free never reads the object, so it settles storage both
/// before and after it holds a `T` ([`CBox::new_uninit`], [`CBox::new_zeroed`]). It
/// releases no sub-resources, so a `T` owning any is promoted to a fuller destructor
/// with `with_policy` before it acquires them.
///
/// A type aligned beyond what the allocator vouches for is rejected:
///
/// ```compile_fail,E0080
/// use libgit2::libgit2::raw::OdbDataBox;
///
/// #[repr(C, align(64))]
/// pub struct wide_st { pub x: u8 }
/// ffibox::define_ctype!(Wide, WideRef, WideMut, wide_st);
///
/// let _ = OdbDataBox::<Wide>::new_uninit();
/// ```
///
/// Zeroed storage needs a type that vouches all-zero is valid:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::raw::OdbDataBox;
///
/// #[repr(C)]
/// pub struct frame_st { pub format: i32 }
/// ffibox::define_ctype!(Frame, FrameRef, FrameMut, frame_st); // not `CZeroable`
///
/// let _ = OdbDataBox::<Frame>::new_zeroed();
/// ```
///
/// Wraps: git_odb_backend_data_alloc
/// Wraps: git_odb_backend_data_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataFree;

/// One `T` in libgit2's allocator.
pub type OdbDataBox<T> = CBox<T, OdbDataFree>;

// SAFETY: `git_odb_backend_data_free` releases the allocation exactly once, without
// reading it, whether or not it holds a `T`; `self` carries no state.
unsafe impl<T> CDrop<T> for OdbDataFree {
    #[inline]
    unsafe fn c_drop(&self, ptr: NonNull<T>) {
        // SAFETY: per `c_drop`'s contract `ptr` is a live allocation whose claim the
        // caller owns; every owner under this policy holds `git__malloc` storage.
        unsafe { data_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `c_alloc` returns fresh, uniquely-owned storage of at least `size_of::<T>()`
// bytes, aligned to `DATA_ALIGN >= align_of::<T>()` (checked at compile time), which
// `c_drop` above releases without reading.
unsafe impl<T: CCell> CAlloc<T> for OdbDataFree {
    #[inline]
    fn c_alloc(&self) -> Option<NonNull<MaybeUninit<T>>> {
        const { assert!(align_of::<T>() <= DATA_ALIGN, "libgit2's allocator cannot align T") };
        data_alloc(size_of::<T>()).map(NonNull::cast)
    }
}

// SAFETY: as `CAlloc`, with every byte of the `T` zeroed before it is handed out, a
// state `T: CZeroable` vouches for; `c_drop` accepts any `T` since it never reads it.
unsafe impl<T: CZeroable> CAllocZeroed<T> for OdbDataFree {
    #[inline]
    fn c_alloc_zeroed(&self) -> Option<NonNull<T>> {
        const { assert!(align_of::<T>() <= DATA_ALIGN, "libgit2's allocator cannot align T") };
        let data = data_alloc(size_of::<T>())?;
        // SAFETY: `data` is a fresh allocation of at least `size_of::<T>()` bytes.
        unsafe { data.as_ptr().write_bytes(0, size_of::<T>()) };
        Some(data.cast())
    }
}

/// A counted run of elements in libgit2's allocator, released through
/// `git_odb_backend_data_free`; the buffer a backend's `read` callback hands libgit2.
///
/// [`CVec::from_slice`] and `Clone` copy into the allocator; [`new_uninit`] and
/// [`new_zeroed`] allocate in place. An empty buffer is NULL and allocates nothing.
///
/// Element types aligned beyond what the allocator vouches for are rejected, as are
/// zero-sized ones, which need no storage:
///
/// ```compile_fail,E0080
/// use libgit2::libgit2::raw::OdbDataVec;
///
/// #[derive(Clone, Copy)]
/// #[repr(align(64))]
/// struct Wide(u8);
///
/// let _ = OdbDataVec::<Wide>::from_slice(&[Wide(0)]);
/// ```
///
/// ```compile_fail,E0080
/// let _ = libgit2::libgit2::raw::OdbDataArrayFree::new_uninit::<()>(4);
/// ```
///
/// [`new_uninit`]: Self::new_uninit
/// [`new_zeroed`]: Self::new_zeroed
///
/// Wraps: git_odb_backend_data_alloc
/// Wraps: git_odb_backend_data_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataArrayFree;

/// A counted run of `T` in libgit2's allocator.
pub type OdbDataVec<T> = CVec<T, OdbDataArrayFree>;

/// `git_odb_backend_data_free` in `CLenDrop`'s shape: the free takes no size.
///
/// # Safety
///
/// As [`data_free`].
unsafe fn data_free_len(data: *mut u8, _byte_len: usize) {
    // SAFETY: forwarded from `c_drop_len`'s contract: a live allocation the caller owns.
    unsafe { data_free(data) }
}

/// A `memdup` into libgit2's allocator: a fresh copy of the `byte_len` bytes at
/// `src`, or NULL on failure. `git__allocator` has no copy routine of its own.
///
/// # Safety
///
/// `src` must be valid for reads of `byte_len` bytes.
unsafe fn data_dup_len(src: *mut u8, byte_len: usize) -> *mut u8 {
    let Some(copy) = data_alloc(byte_len) else {
        return ptr::null_mut();
    };
    // SAFETY: the caller guarantees `byte_len` readable bytes at `src`; `copy` is a
    // fresh allocation of at least `byte_len` bytes, so the two cannot overlap.
    unsafe { ptr::copy_nonoverlapping(src, copy.as_ptr(), byte_len) };
    copy.as_ptr()
}

impl_clendrop!(OdbDataArrayFree, data_free_len);
impl_clenclone!(OdbDataArrayFree, data_dup_len, align = DATA_ALIGN);

impl OdbDataArrayFree {
    /// `len` uninitialised elements; `None` when the allocation fails or its size
    /// overflows. An empty request allocates nothing.
    pub fn new_uninit<T>(len: usize) -> Option<OdbDataVec<MaybeUninit<T>>> {
        let data = Self::alloc_run::<T>(len)?;
        // SAFETY: `data` is NULL with `len == 0`, or fresh, uniquely-owned storage of
        // at least `len * size_of::<T>()` bytes aligned for `T`, which
        // `data_free_len` releases whatever the length; `MaybeUninit<T>` needs no
        // initialisation.
        unsafe { CVec::from_raw_parts_or_empty(data.cast(), len) }
    }

    /// `len` zeroed elements; `None` when the allocation fails or its size
    /// overflows. An empty request allocates nothing.
    pub fn new_zeroed<T: CPlainElem>(len: usize) -> Option<OdbDataVec<T>> {
        let data = Self::alloc_run::<T>(len)?;
        if !data.is_null() {
            // SAFETY: `data` is a fresh allocation of at least `len * size_of::<T>()`
            // bytes.
            unsafe { data.write_bytes(0, len) };
        }
        // SAFETY: as in `new_uninit`, and every element is zero, a valid `T` since
        // `T: CPlainElem` admits every bit pattern.
        unsafe { CVec::from_raw_parts_or_empty(data, len) }
    }

    /// Storage for `len` elements of `T`: NULL for none, `None` on failure.
    fn alloc_run<T>(len: usize) -> Option<*mut T> {
        const {
            assert!(size_of::<T>() != 0, "zero-sized elements need no storage");
            assert!(align_of::<T>() <= DATA_ALIGN, "libgit2's allocator cannot align T");
        }
        if len == 0 {
            return Some(ptr::null_mut());
        }
        // A slice spans at most `isize::MAX` bytes.
        let byte_len = len
            .checked_mul(size_of::<T>())
            .filter(|&n| n <= isize::MAX as usize)?;
        data_alloc(byte_len).map(|data| data.as_ptr().cast())
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use std::ffi::{c_char, c_int, c_void};
    use std::process::Command;

    #[repr(C)]
    pub struct pair_st {
        a: u64,
        b: u32,
    }
    ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);

    impl PairRef<'_> {
        fn a(&self) -> u64 {
            // SAFETY: a read through the shared handle's pointer to a live `pair_st`.
            unsafe { ptr::addr_of!((*self.as_ptr()).a).read() }
        }
        fn b(&self) -> u32 {
            // SAFETY: as above.
            unsafe { ptr::addr_of!((*self.as_ptr()).b).read() }
        }
    }

    /// The raw C calls below need the runtime the strategies pin.
    fn init() {
        assert!(runtime_pinned());
    }

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

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// The one request size the refusing allocator turns down.
    const REFUSED: usize = 12_345;

    unsafe extern "C" fn refusing_malloc(n: usize, _: *const c_char, _: c_int) -> *mut c_void {
        // SAFETY: plain libc allocation.
        if n == REFUSED { ptr::null_mut() } else { unsafe { malloc(n) } }
    }

    unsafe extern "C" fn libc_realloc(p: *mut c_void, n: usize, _: *const c_char, _: c_int) -> *mut c_void {
        // SAFETY: `p` came from this allocator, i.e. libc, or is NULL.
        unsafe { realloc(p, n) }
    }

    unsafe extern "C" fn libc_free(p: *mut c_void) {
        // SAFETY: as above.
        unsafe { free(p) }
    }

    /// Corrects C: `git_odb_backend_data_alloc` before `git_libgit2_init` reports its
    /// failure through TLS key 0, which ASan flags as a wild write.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failure_before_init_lands_on_live_thread_state() {
        isolated(concat!(module_path!(), "::failure_before_init_lands_on_live_thread_state"), || {
            let mut allocator = ffi::git_allocator {
                gmalloc: Some(refusing_malloc),
                grealloc: Some(libc_realloc),
                gfree: Some(libc_free),
            };
            // SAFETY: a `git_allocator *` with every member set, which libgit2 copies;
            // nothing is allocated yet, so no storage changes allocator.
            let rc = unsafe {
                ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR as c_int, &raw mut allocator)
            };
            assert_eq!(rc, 0);

            assert!(OdbDataArrayFree::new_uninit::<u8>(REFUSED).is_none());
            // SAFETY: `git_error_last` never returns NULL.
            let klass = unsafe { (*ffi::git_error_last()).klass };
            assert_eq!(klass, ffi::GIT_ERROR_NOMEMORY as c_int, "reported on this thread's state");
            // SAFETY: no arguments; the extra reference dies with the process.
            assert!(unsafe { ffi::git_libgit2_init() } >= 2, "the strategy holds a reference");
            assert!(OdbDataVec::<u8>::from_slice(b"pinned").is_some());
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn vec_adopts_a_c_allocation() {
        init();
        // SAFETY: `backend` is unused by the routine.
        let raw = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 4) }.cast::<u8>();
        assert!(!raw.is_null());
        // SAFETY: `raw` holds 4 writable bytes.
        unsafe { raw.copy_from_nonoverlapping(b"odb!".as_ptr(), 4) };
        // SAFETY: 4 initialised bytes of fresh `git__malloc` storage, now owned here.
        let v = unsafe { OdbDataVec::<u8>::from_raw_parts(raw, 4) }.unwrap();
        assert_eq!(v.as_slice(), b"odb!");
        // `git_odb_backend_data_free` runs here; ASan flags a mismatch or a leak.
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn vec_into_raw_parts_is_released_by_c() {
        init();
        let v = OdbDataVec::<u16>::from_slice(&[1, 2, 3]).unwrap();
        let (data, len) = v.into_raw_parts();
        assert_eq!(len, 3);
        // SAFETY: `data` is the `git__malloc` buffer `v` gave up, released once here.
        unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), data.cast()) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn vec_uninit_promoted_after_filling() {
        init();
        let mut v = OdbDataArrayFree::new_uninit::<u32>(4).unwrap();
        for (i, slot) in v.as_mut_slice().iter_mut().enumerate() {
            slot.write(i as u32 * 7);
        }
        // SAFETY: every element was written above.
        let v = unsafe { v.assume_init() };
        assert_eq!(v.as_slice(), &[0, 7, 14, 21]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn box_uninit_filled_through_the_c_pointer() {
        init();
        let mut slot = OdbDataBox::<Pair>::new_uninit().unwrap();
        let c = slot.as_c_ptr();
        // SAFETY: `c` addresses fresh storage for one `pair_st`, written whole.
        unsafe { c.write(pair_st { a: u64::MAX, b: 9 }) };
        // SAFETY: the `pair_st` was fully written above and owns no resources.
        let pair = unsafe { slot.assume_init() };
        assert_eq!((pair.as_ref().a(), pair.as_ref().b()), (u64::MAX, 9));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn box_adopts_and_returns_a_c_allocation() {
        init();
        // SAFETY: `backend` is unused by the routine.
        let raw = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), size_of::<pair_st>()) }
            .cast::<pair_st>();
        assert!(!raw.is_null());
        // SAFETY: `raw` is fresh storage for one `pair_st`, malloc-aligned.
        unsafe { raw.write(pair_st { a: 1, b: 2 }) };
        // SAFETY: an initialised `pair_st` in `git__malloc` storage, now owned here.
        let pair = unsafe { OdbDataBox::<Pair>::from_c(raw) }.unwrap();
        assert_eq!(pair.as_ref().b(), 2);
        let raw = pair.into_c();
        // SAFETY: the storage `pair` gave up, released once here.
        unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), raw.cast()) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn dup_copies_and_never_asks_for_zero_bytes() {
        init();
        let mut src = *b"payload";
        // SAFETY: `src` is readable for its length.
        let copy = unsafe { data_dup_len(src.as_mut_ptr(), src.len()) };
        assert!(!copy.is_null() && copy != src.as_mut_ptr());
        // SAFETY: `copy` holds `src.len()` bytes copied from `src`.
        assert_eq!(unsafe { core::slice::from_raw_parts(copy, src.len()) }, &src);
        // SAFETY: `copy` is a live allocation owned here.
        unsafe { data_free(copy) };

        // SAFETY: zero bytes are trivially readable.
        let empty = unsafe { data_dup_len(src.as_mut_ptr(), 0) };
        assert!(!empty.is_null(), "a zero-byte copy is a fresh allocation, not a failure");
        // SAFETY: as above.
        unsafe { data_free(empty) };
    }

    #[test]
    fn oversized_runs_are_rejected_before_allocating() {
        assert!(OdbDataArrayFree::new_uninit::<u64>(usize::MAX).is_none());
        assert!(OdbDataArrayFree::new_zeroed::<u16>(isize::MAX as usize / 2 + 1).is_none());
    }

    #[test]
    fn empty_runs_allocate_nothing() {
        let v = OdbDataArrayFree::new_uninit::<u8>(0).unwrap();
        assert!(v.is_empty() && v.as_ptr().is_null());
        let z = OdbDataArrayFree::new_zeroed::<u32>(0).unwrap();
        assert!(z.as_ptr().is_null());
        let s = OdbDataVec::<u8>::from_slice(&[]).unwrap();
        assert!(s.clone().as_ptr().is_null());
    }

    #[test]
    fn data_align_covers_libgit2_scalars() {
        assert!(DATA_ALIGN >= align_of::<*const ()>() && DATA_ALIGN >= align_of::<u64>());
        assert!(DATA_ALIGN.is_power_of_two());
    }
}
