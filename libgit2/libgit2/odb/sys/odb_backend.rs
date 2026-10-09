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
//! The same pair settles libgit2's strings: `git_strarray_dispose`,
//! `git_signature_free` and `git_buf_dispose` release them with `git__free`, and
//! `git__strdup` fills `git__malloc` storage. [`OdbDataStrFree`] owns one
//! NUL-terminated string ([`OdbDataStr`]); [`OdbDataStrVec`] a counted run of them,
//! laid out as a `git_strarray`'s `strings` / `count`.
//!
//! Not every string libgit2 hands out is one of its allocations: a `git_buf` owns
//! its `ptr` only while `reserved > 0`, and otherwise points at a static empty
//! string that `git_buf_dispose` leaves alone (`buf.c`); an empty result is filled
//! without allocating. [`BufMut::take_str`] applies that rule, so a `git_buf`'s `ptr`
//! is never adopted through `from_raw` directly.
//!
//! [`BufMut::take_str`]: crate::libgit2::core::buffer::BufMut::take_str
//!
//! The first allocation pins libgit2's runtime with a `git_libgit2_init` reference
//! of its own, held for the rest of the process (`runtime_pinned`, from
//! `util::sys::errors`). C callers get NULL before initialisation instead, through a
//! path that corrupts foreign thread-local state; the strategies deliberately diverge
//! from it.
//!
//! Storage handed out by a custom `GIT_OPT_SET_ALLOCATOR` allocator is released
//! through whichever allocator is current when the owner drops, so the allocator must
//! not be replaced while an owner is live — the contract of that option, not of these
//! policies.

use core::ffi::{CStr, c_char};
use core::fmt;
use core::mem::{ManuallyDrop, MaybeUninit, align_of, size_of};
use core::ptr::{self, NonNull};

use ffibox::{
    CAlloc, CAllocZeroed, CBox, CCell, CDrop, CPlainElem, CStrBox, CVec, CZeroable,
    impl_cdrop_str, impl_cdupclone_str, impl_clenclone, impl_clendrop,
};

use crate::ffi;
use crate::libgit2::util::sys::errors::runtime_pinned;

/// The alignment `git_odb_backend_data_alloc` storage is guaranteed to have.
///
/// It is `git__allocator.gmalloc` storage, in which libgit2 itself places structs
/// holding pointers, `size_t`s and 64-bit integers, so any allocator libgit2 runs on
/// aligns that far. The default `stdalloc` is glibc's `malloc`, which aligns to 16.
const DATA_ALIGN: usize = {
    let (word, wide) = (align_of::<usize>(), align_of::<u64>());
    if word > wide { word } else { wide }
};

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

/// A copy of `s` and its terminator in libgit2's allocator: `git__strdup` over the
/// exported routines. `None` on failure.
fn str_dup(s: &CStr) -> Option<NonNull<c_char>> {
    let bytes = s.to_bytes_with_nul();
    let copy = data_alloc(bytes.len())?;
    // SAFETY: `copy` is a fresh allocation of at least `bytes.len()` bytes, so it
    // cannot overlap the borrowed `bytes`.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), copy.as_ptr(), bytes.len()) };
    Some(copy.cast())
}

/// `git_odb_backend_data_free` in `impl_cdrop_str!`'s shape.
///
/// # Safety
///
/// As [`data_free`].
unsafe fn str_free(s: *mut c_char) {
    // SAFETY: forwarded from `c_drop`'s contract: a live string the caller owns.
    unsafe { data_free(s.cast()) }
}

/// `str_dup` in `impl_cdupclone_str!`'s shape: a fresh copy, or NULL on failure.
///
/// # Safety
///
/// `s` must be a live NUL-terminated string.
unsafe fn str_dup_raw(s: *mut c_char) -> *mut c_char {
    // SAFETY: per this function's contract `s` is NUL-terminated and stays live for
    // the call; the copy does not write through it.
    let s = unsafe { CStr::from_ptr(s) };
    str_dup(s).map_or(ptr::null_mut(), NonNull::as_ptr)
}

/// One NUL-terminated string in libgit2's allocator, released through
/// `git_odb_backend_data_free`: the strings `git_strarray_dispose`,
/// `git_signature_free` and `git_buf_dispose` release with `git__free`.
///
/// [`dup`](Self::dup) and `Clone` copy into the allocator, as `git__strdup` does;
/// [`CStrBox::into_raw`] hands the string to C, which may release it with any of
/// those routines. A string libgit2 made is adopted with [`CStrBox::from_raw`] only
/// when libgit2 owns it: a `git_strarray` element or a `git_signature` name always
/// is, a `git_buf`'s `ptr` only while `reserved > 0` — [`BufMut::take_str`] checks
/// that, where `from_raw` would free libgit2's static empty string.
///
/// [`BufMut::take_str`]: crate::libgit2::core::buffer::BufMut::take_str
///
/// Wraps: git_odb_backend_data_alloc
/// Wraps: git_odb_backend_data_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct OdbDataStrFree;

/// One NUL-terminated string in libgit2's allocator.
pub type OdbDataStr = CStrBox<OdbDataStrFree>;

impl_cdrop_str!(OdbDataStrFree, str_free);
impl_cdupclone_str!(OdbDataStrFree, str_dup_raw);

impl OdbDataStrFree {
    /// A copy of `s` in libgit2's allocator; `None` when the allocation fails.
    pub fn dup(s: &CStr) -> Option<OdbDataStr> {
        let copy = str_dup(s)?;
        // SAFETY: `copy` is fresh, uniquely-owned `git__malloc` storage holding `s`
        // with its terminator, which `str_free` releases.
        unsafe { OdbDataStr::from_raw(copy.as_ptr()) }
    }
}

// An `OdbDataStr` is a non-null `char *`: its policy is zero-sized, so a run of them
// is a `char **` array with no NULL element.
const _: () = assert!(
    size_of::<OdbDataStr>() == size_of::<*mut c_char>()
        && align_of::<OdbDataStr>() == align_of::<*mut c_char>()
);

/// A counted run of strings in libgit2's allocator: the `strings` / `count` pair of a
/// `git_strarray`, which `git_strarray_dispose` releases string by string, then as a
/// whole, with `git__free`.
///
/// Each element is an [`OdbDataStr`] under [`OdbDataStrFree`], and the run itself is
/// [`OdbDataVec`] storage under [`OdbDataArrayFree`]; dropping it frees every string
/// in order, then the array, through `git_odb_backend_data_free`. No element is NULL.
/// An empty run is NULL and allocates nothing.
///
/// [`dup_all`](Self::dup_all) and `Clone` copy every string, as `git_strarray_copy`
/// does; [`from_vec`](Self::from_vec) and [`into_vec`](Self::into_vec) move them
/// between the run and a `Vec`, which is where a run grows or shrinks.
///
/// Wraps: git_odb_backend_data_alloc
/// Wraps: git_odb_backend_data_free
/// Handwritten
#[repr(transparent)]
pub struct OdbDataStrVec(OdbDataVec<OdbDataStr>);

impl OdbDataStrVec {
    /// An empty run: NULL, nothing to free.
    #[must_use]
    pub const fn new() -> Self {
        Self(CVec::empty_with(OdbDataArrayFree))
    }

    /// Copies of `strs`, in order; `None` when an allocation fails, after releasing
    /// whatever was copied. An empty `strs` allocates nothing.
    pub fn dup_all<S: AsRef<CStr>>(strs: &[S]) -> Option<Self> {
        Self::dup_each(strs, |s| s.as_ref())
    }

    /// `strs` moved into a run in libgit2's allocator; `None`, dropping them, when the
    /// array cannot be allocated.
    pub fn from_vec(strs: Vec<OdbDataStr>) -> Option<Self> {
        let mut run = OdbDataArrayFree::new_uninit::<OdbDataStr>(strs.len())?;
        for (slot, s) in run.as_mut_slice().iter_mut().zip(strs) {
            slot.write(s);
        }
        // SAFETY: the run has `strs.len()` slots and the loop wrote each of them.
        Some(Self(unsafe { run.assume_init() }))
    }

    /// The strings moved out into a `Vec`, in order; the array is released.
    #[must_use]
    pub fn into_vec(self) -> Vec<OdbDataStr> {
        let mut out = Vec::with_capacity(self.len());
        let (strings, count) = self.into_storage().into_raw_parts();
        for i in 0..count {
            // SAFETY: `strings` holds `count` live strings that the run gave up; each
            // is moved out exactly once.
            out.push(unsafe { strings.add(i).read() });
        }
        if !strings.is_null() {
            // SAFETY: `strings` is the run's `git__malloc` array, now emptied of its
            // owners and released once.
            unsafe { data_free(strings.cast()) };
        }
        out
    }

    /// Takes ownership of `count` strings at `strings`, a `git_strarray`'s pair.
    ///
    /// NULL with `count == 0` is the empty run, as in a zeroed `git_strarray`. Returns
    /// `None`, leaving ownership with the caller, when `strings` is NULL with a
    /// nonzero `count` or any of its elements is NULL — which `git_strarray_copy`
    /// would silently skip; libgit2's own outputs hold none.
    ///
    /// # Safety
    ///
    /// A non-null `strings` must address `count` initialised `char *` elements in an
    /// array allocated by libgit2's current allocator — as `git_strarray_dispose`
    /// expects, possibly with spare capacity — and each non-null element a
    /// NUL-terminated string from the same allocator. The caller transfers unique
    /// ownership of the array and of every string, and does not use them again unless
    /// `None` is returned.
    pub unsafe fn from_raw_parts(strings: *mut *mut c_char, count: usize) -> Option<Self> {
        if strings.is_null() {
            return (count == 0).then(Self::new);
        }
        for i in 0..count {
            // SAFETY: per this function's contract `strings` holds `count` initialised
            // elements.
            if unsafe { strings.add(i).read() }.is_null() {
                return None;
            }
        }
        // SAFETY: `strings` is a `git__malloc` array of `count` non-null strings from
        // the same allocator, uniquely owned by the caller, who hands them over; an
        // `OdbDataStr` is a non-null `char *` that `str_free` releases.
        let run = unsafe { OdbDataVec::from_raw_parts(strings.cast::<OdbDataStr>(), count) };
        run.map(Self)
    }

    /// Gives up the run without freeing: the `strings` / `count` pair to place in a
    /// `git_strarray`, which `git_strarray_dispose` then releases.
    #[must_use = "the returned pointer owns the strings and must be freed"]
    pub fn into_raw_parts(self) -> (*mut *mut c_char, usize) {
        let (strings, count) = self.into_storage().into_raw_parts();
        (strings.cast(), count)
    }

    /// The array, for passing to C; NULL when empty. Ownership is retained.
    #[must_use]
    pub fn as_ptr(&self) -> *mut *mut c_char {
        self.0.as_ptr().cast()
    }

    /// Number of strings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the run holds no strings.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The strings.
    #[must_use]
    pub fn as_slice(&self) -> &[OdbDataStr] {
        // SAFETY: the run owns `len` live, non-null strings at `data()` (none, at an
        // aligned dangling pointer, when NULL); the slice is bound by `&self`.
        unsafe { core::slice::from_raw_parts(self.data(), self.len()) }
    }

    /// The strings, mutably: an element replaced through it drops the old string.
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [OdbDataStr] {
        // SAFETY: as `as_slice`, with `&mut self` making it exclusive.
        unsafe { core::slice::from_raw_parts_mut(self.data(), self.len()) }
    }

    /// Copies every string; `None` on failure. See `Clone`.
    pub fn try_clone(&self) -> Option<Self> {
        Self::dup_each(self.as_slice(), OdbDataStr::as_c_str)
    }

    /// The elements' base for Rust-side views: dangling when the run is NULL.
    fn data(&self) -> *mut OdbDataStr {
        let strings = self.0.as_ptr();
        if strings.is_null() {
            NonNull::dangling().as_ptr()
        } else {
            strings
        }
    }

    /// The storage, its strings no longer dropped with the run.
    fn into_storage(self) -> OdbDataVec<OdbDataStr> {
        let this = ManuallyDrop::new(self);
        // SAFETY: the storage is read out exactly once and `self` is never dropped.
        unsafe { ptr::read(&this.0) }
    }

    /// Copies of every item's string in `git_strarray_copy`'s order: the array first,
    /// then each string. A failed copy releases the copies made so far, then the
    /// array, as `git_strarray_dispose` does there.
    fn dup_each<T>(items: &[T], as_c_str: impl Fn(&T) -> &CStr) -> Option<Self> {
        let mut run = OdbDataArrayFree::new_uninit::<OdbDataStr>(items.len())?;
        let slots = run.as_mut_slice();
        for (i, item) in items.iter().enumerate() {
            let Some(copy) = OdbDataStrFree::dup(as_c_str(item)) else {
                for slot in &mut slots[..i] {
                    // SAFETY: the slots before `i` were written below, and each is
                    // dropped once, before the storage is released.
                    unsafe { slot.assume_init_drop() };
                }
                return None;
            };
            slots[i].write(copy);
        }
        // SAFETY: the run has `items.len()` slots and the loop wrote each of them.
        Some(Self(unsafe { run.assume_init() }))
    }
}

impl Drop for OdbDataStrVec {
    fn drop(&mut self) {
        // SAFETY: the run owns its strings, each dropped once here; the storage, which
        // the inner `OdbDataVec` frees right after, is not read again.
        unsafe { ptr::drop_in_place(self.as_mut_slice()) }
    }
}

impl Clone for OdbDataStrVec {
    /// Copies every string; aborts when an allocation fails, as `OdbDataStr`'s
    /// `Clone` does. [`try_clone`](Self::try_clone) reports the failure instead.
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|| std::process::abort())
    }
}

impl Default for OdbDataStrVec {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for OdbDataStrVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.as_slice().iter().map(OdbDataStr::as_c_str))
            .finish()
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use std::ffi::{c_char, c_int, c_void};
    use std::process::Command;
    use std::sync::atomic::Ordering::SeqCst;
    use std::sync::atomic::{AtomicIsize, AtomicUsize};

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
    fn policies_are_stateless_values() {
        // Both policies carry no state, so an owner is pointer-sized (plus length)
        // and the policy can be named, copied and printed freely.
        assert_eq!(size_of::<OdbDataBox<Pair>>(), size_of::<*mut pair_st>());
        assert_eq!(size_of::<Option<OdbDataBox<Pair>>>(), size_of::<*mut pair_st>());
        assert_eq!(size_of::<OdbDataVec<u8>>(), 2 * size_of::<usize>());
        let (one, run) = (OdbDataFree, OdbDataArrayFree);
        let (copy_one, copy_run) = (one, run);
        assert_eq!(format!("{one:?}{copy_one:?}"), "OdbDataFreeOdbDataFree");
        assert_eq!(format!("{run:?}{copy_run:?}"), "OdbDataArrayFreeOdbDataArrayFree");
        let v = OdbDataVec::<u16>::default();
        assert!(v.is_empty() && v.as_ptr().is_null());
        assert_eq!(format!("{v:?}"), "CVec { ptr: 0x0, len: 0 }");
        let (ptr, len) = OdbDataVec::<u16>::empty_with(copy_run).into_raw_parts();
        assert!(ptr.is_null() && len == 0);
    }

    #[test]
    fn data_align_covers_libgit2_scalars() {
        assert!(DATA_ALIGN >= align_of::<*const ()>() && DATA_ALIGN >= align_of::<u64>());
        assert!(DATA_ALIGN.is_power_of_two());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_dup_copies_the_terminator() {
        init();
        let copy = str_dup(c"refs/heads/main").unwrap();
        // SAFETY: `copy` holds the 16 bytes copied from the literal, terminator included.
        let bytes = unsafe { core::slice::from_raw_parts(copy.as_ptr().cast::<u8>(), 16) };
        assert_eq!(bytes, b"refs/heads/main\0");
        // SAFETY: `copy` is a live allocation owned here.
        unsafe { str_free(copy.as_ptr()) };

        let empty = str_dup(c"").unwrap();
        // SAFETY: `empty` holds one byte, the terminator.
        assert_eq!(unsafe { empty.as_ptr().read() }, 0);
        // SAFETY: as above.
        unsafe { str_free(empty.as_ptr()) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_dup_raw_matches_the_source() {
        init();
        let mut src = *b"user.name\0";
        // SAFETY: `src` is NUL-terminated and outlives the call.
        let copy = unsafe { str_dup_raw(src.as_mut_ptr().cast()) };
        assert!(!copy.is_null() && copy != src.as_mut_ptr().cast());
        // SAFETY: `copy` is a live NUL-terminated string.
        assert_eq!(unsafe { CStr::from_ptr(copy) }, c"user.name");
        // SAFETY: owned here, released once.
        unsafe { str_free(copy) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_adopts_a_c_allocation_and_returns_it() {
        init();
        // SAFETY: `backend` is unused by the routine.
        let raw = unsafe { ffi::git_odb_backend_data_alloc(ptr::null_mut(), 4) }.cast::<c_char>();
        assert!(!raw.is_null());
        // SAFETY: `raw` holds 4 writable bytes.
        unsafe { raw.copy_from_nonoverlapping(c"odb".as_ptr(), 4) };
        // SAFETY: a NUL-terminated string in fresh `git__malloc` storage, now owned here.
        let s = unsafe { OdbDataStr::from_raw(raw) }.unwrap();
        assert_eq!(s.to_str(), Ok("odb"));
        let t = s.clone();
        assert_ne!(s.as_ptr(), t.as_ptr());
        let raw = s.into_raw();
        // SAFETY: the string `s` gave up, released once here.
        unsafe { ffi::git_odb_backend_data_free(ptr::null_mut(), raw.cast()) };
        assert_eq!(t.as_bytes(), b"odb");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_vec_into_vec_releases_only_the_array() {
        init();
        let v = OdbDataStrVec::dup_all(&[c"a", c"bc", c"def"]).unwrap();
        let ptrs: Vec<_> = v.as_slice().iter().map(OdbDataStr::as_ptr).collect();
        let strs = v.into_vec();
        assert_eq!(
            strs.iter().map(OdbDataStr::as_ptr).collect::<Vec<_>>(),
            ptrs
        );
        assert_eq!(strs[2].as_bytes(), b"def");
        let back = OdbDataStrVec::from_vec(strs).unwrap();
        assert_eq!(
            back.as_slice()
                .iter()
                .map(OdbDataStr::as_ptr)
                .collect::<Vec<_>>(),
            ptrs
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_vec_adopts_its_own_raw_parts() {
        init();
        let v = OdbDataStrVec::dup_all(&[c"x", c"y"]).unwrap();
        let (strings, count) = v.into_raw_parts();
        // SAFETY: the pair `v` gave up: a `git__malloc` array of two owned strings.
        let w = unsafe { OdbDataStrVec::from_raw_parts(strings, count) }.unwrap();
        assert_eq!(w.as_ptr(), strings);
        assert_eq!(format!("{w:?}"), r#"["x", "y"]"#);
    }

    static LIVE: AtomicIsize = AtomicIsize::new(0);
    /// Fails the `n`th allocation from now when `n > 0`.
    static FAIL_IN: AtomicUsize = AtomicUsize::new(0);

    /// Counts `FAIL_IN` down; `true` for the allocation it names.
    fn refuse_now() -> bool {
        let mut k = FAIL_IN.load(SeqCst);
        while k > 0 {
            match FAIL_IN.compare_exchange(k, k - 1, SeqCst, SeqCst) {
                Ok(_) => return k == 1,
                Err(now) => k = now,
            }
        }
        false
    }

    unsafe extern "C" fn counting_malloc(n: usize, _: *const c_char, _: c_int) -> *mut c_void {
        if refuse_now() {
            return ptr::null_mut();
        }
        LIVE.fetch_add(1, SeqCst);
        // SAFETY: plain libc allocation.
        unsafe { malloc(n) }
    }

    unsafe extern "C" fn counting_free(p: *mut c_void) {
        if !p.is_null() {
            LIVE.fetch_sub(1, SeqCst);
        }
        // SAFETY: `p` came from this allocator, i.e. libc, or is NULL.
        unsafe { free(p) }
    }

    /// A copy failing at any allocation releases the copies made so far and the
    /// array, and a failing `try_clone` leaves its source intact.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn str_vec_failed_copy_releases_what_it_made() {
        isolated(
            concat!(
                module_path!(),
                "::str_vec_failed_copy_releases_what_it_made"
            ),
            || {
                let mut allocator = ffi::git_allocator {
                    gmalloc: Some(counting_malloc),
                    grealloc: Some(libc_realloc),
                    gfree: Some(counting_free),
                };
                // SAFETY: a `git_allocator *` with every member set, which libgit2 copies;
                // nothing is allocated yet, and nothing reallocates between the counts.
                let rc = unsafe {
                    ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR as c_int, &raw mut allocator)
                };
                assert_eq!(rc, 0);
                init();
                // The first failure allocates this thread's error state; count after it.
                FAIL_IN.store(1, SeqCst);
                assert!(OdbDataStrFree::dup(c"warm").is_none());

                let strs = [c"one", c"two", c"three"];
                let before = LIVE.load(SeqCst);
                for at in 1..=4 {
                    FAIL_IN.store(at, SeqCst);
                    assert!(
                        OdbDataStrVec::dup_all(&strs).is_none(),
                        "allocation {at} fails"
                    );
                    assert_eq!(LIVE.load(SeqCst), before, "allocation {at} fails");
                }

                let v = OdbDataStrVec::dup_all(&strs).unwrap();
                let held = LIVE.load(SeqCst);
                assert_eq!(held - before, 4);
                for at in 1..=4 {
                    FAIL_IN.store(at, SeqCst);
                    assert!(v.try_clone().is_none());
                    assert_eq!(LIVE.load(SeqCst), held);
                }
                FAIL_IN.store(1, SeqCst);
                assert!(v.as_slice()[1].try_clone().is_none());
                assert_eq!(format!("{v:?}"), r#"["one", "two", "three"]"#);
                drop(v);
                assert_eq!(LIVE.load(SeqCst), before);
            },
        );
    }

    /// `Clone` on a failed copy aborts the process, as documented, for a string and
    /// for a run; nothing is handed back.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn clone_aborts_when_the_copy_fails() {
        use std::os::unix::process::ExitStatusExt;

        const ABORTING: &str = "CRUSTIFY_UNIT_ABORTING";
        if let Ok(which) = std::env::var(ABORTING) {
            let mut allocator = ffi::git_allocator {
                gmalloc: Some(counting_malloc),
                grealloc: Some(libc_realloc),
                gfree: Some(counting_free),
            };
            // SAFETY: a `git_allocator *` with every member set, which libgit2 copies;
            // nothing is allocated yet.
            let rc = unsafe {
                ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR as c_int, &raw mut allocator)
            };
            assert_eq!(rc, 0);
            init();
            let s = OdbDataStrFree::dup(c"keep").unwrap();
            let v = OdbDataStrVec::dup_all(&[c"a", c"b"]).unwrap();
            // Warm this thread's error state, which the first failure allocates.
            FAIL_IN.store(1, SeqCst);
            assert!(s.try_clone().is_none());
            FAIL_IN.store(1, SeqCst);
            let _ = match which.as_str() {
                "str" => s.clone().len(),
                "vec" => v.clone().len(),
                _ => unreachable!(),
            };
            unreachable!("clone returned");
        }
        for which in ["str", "vec"] {
            let name = concat!(module_path!(), "::clone_aborts_when_the_copy_fails");
            let name = name.split_once("::").map_or(name, |(_, rest)| rest);
            let out = Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(ABORTING, which)
                .output()
                .unwrap();
            assert_eq!(
                out.status.signal(),
                Some(SIGABRT),
                "`{which}` clone: {:?}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    const SIGABRT: i32 = 6;

    #[test]
    fn str_vec_empty_forms_own_nothing() {
        let v = OdbDataStrVec::new();
        assert!(v.is_empty() && v.as_ptr().is_null() && v.as_slice().is_empty());
        assert_eq!(format!("{v:?}"), "[]");
        let mut d = OdbDataStrVec::default();
        assert!(d.as_mut_slice().is_empty());
        assert!(d.try_clone().unwrap().as_ptr().is_null());
        assert!(v.clone().into_vec().is_empty());
        let (strings, count) = OdbDataStrVec::dup_all::<&CStr>(&[])
            .unwrap()
            .into_raw_parts();
        assert!(strings.is_null() && count == 0);
        assert!(
            OdbDataStrVec::from_vec(Vec::new())
                .unwrap()
                .as_ptr()
                .is_null()
        );
        // SAFETY: NULL with no count is the empty run and adopts nothing.
        let e = unsafe { OdbDataStrVec::from_raw_parts(ptr::null_mut(), 0) }.unwrap();
        assert!(e.is_empty());
        // SAFETY: NULL with a count is refused without reading anything.
        assert!(unsafe { OdbDataStrVec::from_raw_parts(ptr::null_mut(), 3) }.is_none());
    }

    #[test]
    fn str_owners_are_pointer_sized() {
        assert_eq!(size_of::<OdbDataStr>(), size_of::<*mut c_char>());
        assert_eq!(size_of::<Option<OdbDataStr>>(), size_of::<*mut c_char>());
        assert_eq!(size_of::<OdbDataStrVec>(), size_of::<ffi::git_strarray>());
        let policy = OdbDataStrFree;
        let copy = policy;
        assert_eq!(
            format!("{policy:?}{copy:?}"),
            "OdbDataStrFreeOdbDataStrFree"
        );
    }
}
