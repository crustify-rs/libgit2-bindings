//! Safe wrappers over `include/git2/sys/alloc.h`: the memory allocator libgit2 runs on.
//!
//! An [`Allocator`] is a `git_allocator` value: three C function pointers forming one
//! `malloc` / `realloc` / `free` family. `GIT_OPT_SET_ALLOCATOR` copies it into
//! libgit2's hidden global `git__allocator` (`git_allocator_setup` is a `memcpy`), so
//! the value is only borrowed for that call and owns nothing: it is held by value,
//! copied freely, and never released.
//!
//! The header's init routines are not part of libgit2.so's interface:
//! `git_stdalloc_init_allocator` is compiled hidden and
//! `git_win32_crtdbg_init_allocator` is never defined on this platform. A value is
//! therefore built in Rust, from an [`Alloc`] implementation ([`Allocator::new`]),
//! whose `extern "C"` trampolines adapt libgit2's calls to it, or from three C
//! functions ([`Allocator::from_fns`]).
//!
//! The members are function pointers, so reading or replacing one is safe and calling
//! one is `unsafe`, under the contract its type alias states. Which members make a
//! coherent family is a property of the installed value as a whole, so it is the
//! installing call that relies on it, not these accessors.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::define_ctype;

use crate::ffi;

/// `git_allocator.gmalloc`: allocate `n` bytes for the caller at `file`:`line`.
///
/// Returns a fresh allocation of at least `n` bytes, or NULL on failure. A zero-byte
/// request is as C's `malloc(0)`: NULL, or an allocation of its own that `gfree`
/// releases; libgit2 reads NULL as out of memory.
///
/// # Safety
///
/// `file` must be NULL or a NUL-terminated string live for the call (libgit2 passes
/// `__FILE__`). A function may add obligations of its own; the trampolines
/// [`Allocator::new`] stores add none.
pub type MallocFn = unsafe extern "C" fn(n: usize, file: *const c_char, line: c_int) -> *mut c_void;

/// `git_allocator.grealloc`: resize the allocation at `ptr` to `size` bytes for the
/// caller at `file`:`line`, as C's `realloc`.
///
/// NULL `ptr` allocates afresh, as [`MallocFn`] does. On success the old allocation is
/// released and a new one holding its first `min(old, size)` bytes is returned; on
/// failure NULL is returned and `ptr` stays live.
///
/// # Safety
///
/// `ptr` must be NULL or a live allocation of the same allocator family, which the
/// caller owns and, on success, gives up; `file` as for [`MallocFn`].
///
/// `size` must be non-zero when `ptr` is not NULL. C leaves a zero-byte resize of a
/// live allocation implementation-defined: glibc releases `ptr` and returns NULL, which
/// this contract would read as a failure that left `ptr` live. No libgit2 code path
/// makes one, so a caller must not either.
pub type ReallocFn = unsafe extern "C" fn(
    ptr: *mut c_void,
    size: usize,
    file: *const c_char,
    line: c_int,
) -> *mut c_void;

/// `git_allocator.gfree`: release the allocation at `ptr`; NULL is a no-op.
///
/// # Safety
///
/// `ptr` must be NULL or a live allocation of the same allocator family, which the
/// caller owns and does not use again.
pub type FreeFn = unsafe extern "C" fn(ptr: *mut c_void);

define_ctype!(
    /// A custom memory allocator for libgit2: its `malloc`, `realloc` and `free`
    /// (`git_allocator`).
    ///
    /// A plain value of three function pointers, resource-free: `Copy`, held inline,
    /// never released. Build one with [`new`](Self::new) over an [`Alloc`]
    /// implementation, or with [`from_fns`](Self::from_fns) over C functions; both set
    /// every member, and the setters only ever store a function, so a value built here
    /// never holds a NULL member. Read and replace members through
    /// [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut).
    ///
    /// Wraps: git_allocator
    #[derive(Clone, Copy)]
    Allocator,
    AllocatorRef,
    AllocatorMut,
    ffi::git_allocator
);

// SAFETY: an `Allocator` is three function pointers and nothing else: no interior
// mutability, no thread affinity, nothing to release. Moving one to another thread
// moves three code addresses; calling them stays `unsafe`, under their own contract.
unsafe impl Send for Allocator {}

// SAFETY: `AllocatorMut` is an exclusive borrow of such a value, as `&mut Allocator`.
unsafe impl Send for AllocatorMut<'_> {}

// SAFETY: shared access only reads the three pointers (every write goes through
// `AllocatorMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for Allocator {}

// SAFETY: as above; `AllocatorRef` is a shared borrow, as `&Allocator`.
unsafe impl Send for AllocatorRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for AllocatorRef<'_> {}

// SAFETY: `&AllocatorMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for AllocatorMut<'_> {}

impl Allocator {
    /// The allocator over three C functions forming one family.
    ///
    /// Building the value is safe: nothing is called until it is installed, and
    /// installing it is what relies on `malloc`, `realloc` and `free` belonging
    /// together.
    pub const fn from_fns(malloc: MallocFn, realloc: ReallocFn, free: FreeFn) -> Self {
        Self(
            ffi::git_allocator {
                gmalloc: Some(malloc),
                grealloc: Some(realloc),
                gfree: Some(free),
            },
            PhantomData,
        )
    }

    /// The allocator that forwards libgit2's calls to `A`.
    ///
    /// Its members are `extern "C"` trampolines generic over `A`: they turn the call
    /// site's `file` into an `Option<&CStr>`, pointers into `Option<NonNull<u8>>`,
    /// and drop `free(NULL)` before it reaches [`Alloc::free`]. A panic in `A`
    /// aborts the process, since it cannot unwind into C.
    ///
    /// ```
    /// use core::ffi::{CStr, c_void};
    /// use core::ptr::NonNull;
    ///
    /// use libgit2::libgit2::src::util::alloc::{Alloc, Allocator};
    ///
    /// unsafe extern "C" {
    ///     fn malloc(n: usize) -> *mut c_void;
    ///     fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    ///     fn free(p: *mut c_void);
    /// }
    ///
    /// /// libc's allocator, which meets `Alloc`'s contract as C's `malloc` family does
    /// /// (libgit2 never resizes a live allocation to zero bytes, where glibc would
    /// /// release it and return NULL).
    /// struct Libc;
    ///
    /// // SAFETY: forwards to libc's `malloc` family, unchanged.
    /// unsafe impl Alloc for Libc {
    ///     fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
    ///         NonNull::new(unsafe { malloc(len) }.cast())
    ///     }
    ///
    ///     unsafe fn realloc(
    ///         p: Option<NonNull<u8>>,
    ///         len: usize,
    ///         _: Option<&CStr>,
    ///         _: i32,
    ///     ) -> Option<NonNull<u8>> {
    ///         let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
    ///         NonNull::new(unsafe { realloc(p, len) }.cast())
    ///     }
    ///
    ///     unsafe fn free(p: NonNull<u8>) {
    ///         unsafe { free(p.as_ptr().cast()) }
    ///     }
    /// }
    ///
    /// static LIBC: Allocator = Allocator::new::<Libc>();
    /// assert!(LIBC.as_ref().malloc().is_some());
    /// ```
    pub const fn new<A: Alloc>() -> Self {
        Self::from_fns(malloc_with::<A>, realloc_with::<A>, free_with::<A>)
    }
}

impl fmt::Debug for Allocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl AllocatorRef<'_> {
    /// The `malloc` member; `None` only in a value C left unset, never in one built
    /// here.
    ///
    /// Field: `git_allocator.gmalloc`
    #[must_use]
    pub fn malloc(&self) -> Option<MallocFn> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let gmalloc_ptr = unsafe { &raw const (*self.as_ptr()).gmalloc };
        // SAFETY: a read through the shared handle's pointer to a live `git_allocator`;
        // every bit pattern of the member, NULL included, is a valid `Option<fn>`.
        unsafe { gmalloc_ptr.read() }
    }

    /// The `realloc` member; `None` only in a value C left unset.
    ///
    /// Field: `git_allocator.grealloc`
    #[must_use]
    pub fn realloc(&self) -> Option<ReallocFn> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let grealloc_ptr = unsafe { &raw const (*self.as_ptr()).grealloc };
        // SAFETY: as in `malloc`.
        unsafe { grealloc_ptr.read() }
    }

    /// The `free` member; `None` only in a value C left unset.
    ///
    /// Field: `git_allocator.gfree`
    #[must_use]
    pub fn free(&self) -> Option<FreeFn> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let gfree_ptr = unsafe { &raw const (*self.as_ptr()).gfree };
        // SAFETY: as in `malloc`.
        unsafe { gfree_ptr.read() }
    }
}

impl fmt::Debug for AllocatorRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let addr = |p: Option<*const ()>| p.unwrap_or(ptr::null());
        f.debug_struct("Allocator")
            .field("malloc", &addr(self.malloc().map(|p| p as *const ())))
            .field("realloc", &addr(self.realloc().map(|p| p as *const ())))
            .field("free", &addr(self.free().map(|p| p as *const ())))
            .finish()
    }
}

impl AllocatorMut<'_> {
    /// Replaces the `malloc` member.
    ///
    /// Field: `git_allocator.gmalloc`
    pub fn set_malloc(&mut self, malloc: MallocFn) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let gmalloc_ptr = unsafe { &raw mut (*self.as_mut_ptr()).gmalloc };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_allocator`; the member is plain data with nothing to release.
        unsafe { gmalloc_ptr.write(Some(malloc)) }
    }

    /// Replaces the `realloc` member.
    ///
    /// Field: `git_allocator.grealloc`
    pub fn set_realloc(&mut self, realloc: ReallocFn) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let grealloc_ptr = unsafe { &raw mut (*self.as_mut_ptr()).grealloc };
        // SAFETY: as in `set_malloc`.
        unsafe { grealloc_ptr.write(Some(realloc)) }
    }

    /// Replaces the `free` member.
    ///
    /// Field: `git_allocator.gfree`
    pub fn set_free(&mut self, free: FreeFn) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let gfree_ptr = unsafe { &raw mut (*self.as_mut_ptr()).gfree };
        // SAFETY: as in `set_malloc`.
        unsafe { gfree_ptr.write(Some(free)) }
    }
}

/// A memory allocator libgit2 can run on, adapted to `git_allocator`'s members by
/// [`Allocator::new`]: libgit2's counterpart of [`GlobalAlloc`](std::alloc::GlobalAlloc),
/// without layouts, since C passes only sizes.
///
/// `git_allocator` carries no state, so an implementation is a type whose associated
/// functions reach any state they need through statics. `file` and `line` name the
/// libgit2 call site; `file` is borrowed for the call only.
///
/// # Safety
///
/// libgit2 trusts these functions as it trusts C's `malloc`, `realloc` and `free`:
///
/// - `malloc(len, ..)` returns `None`, or the start of a fresh allocation of at least `len` bytes,
///   aligned for any object that fits in `len` bytes as `malloc` aligns (to `max_align_t`, or the
///   largest power of two not above a smaller `len`), overlapping no other live allocation of the
///   implementation. `len` may be 0, as for C's `malloc(0)`: the result is `None` (which libgit2
///   reads as out of memory) or an allocation of its own, which `free` releases like any other;
/// - `realloc(None, len, ..)` behaves as `malloc`. `realloc(Some(ptr), len, ..)`, with `ptr` a live
///   allocation of the implementation and `len` non-zero, either returns `None` and leaves `ptr`
///   live and unchanged, or releases `ptr` and returns an allocation as `malloc` would whose first
///   `min(old, len)` bytes are those `ptr` held (it may be `ptr` itself). It is never called with
///   `Some(ptr)` and `len == 0`, which C leaves implementation-defined (see [`ReallocFn`]), so an
///   implementation need not decide that case;
/// - `free(ptr)` releases `ptr`, a live allocation of the implementation;
/// - all three may run concurrently on any threads, and an allocation may be resized or released on
///   a thread other than the one that made it.
///
/// None of them may call into libgit2: libgit2 allocates through them, its per-thread
/// error state included (`threadstate_get`), so a libgit2 call from inside one recurses
/// without bound.
///
/// Handwritten
pub unsafe trait Alloc {
    /// Allocates at least `len` bytes; `None` on failure.
    fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>>;

    /// Resizes `ptr` to `len` bytes, or allocates afresh for `None`; `None` on
    /// failure, leaving `ptr` live.
    ///
    /// # Safety
    ///
    /// `ptr` is `None` or a live allocation of this implementation that the caller
    /// owns and, on success, gives up; `len` is non-zero when `ptr` is `Some`.
    unsafe fn realloc(
        ptr: Option<NonNull<u8>>,
        len: usize,
        file: Option<&CStr>,
        line: i32,
    ) -> Option<NonNull<u8>>;

    /// Releases `ptr`.
    ///
    /// # Safety
    ///
    /// `ptr` is a live allocation of this implementation that the caller owns and does
    /// not use again.
    unsafe fn free(ptr: NonNull<u8>);
}

/// The call site's file name, as C passes it.
///
/// # Safety
///
/// `file` is NULL or a NUL-terminated string live for `'a`.
unsafe fn site_file<'a>(file: *const c_char) -> Option<&'a CStr> {
    // SAFETY: per this function's contract, a non-NULL `file` is a live C string.
    (!file.is_null()).then(|| unsafe { CStr::from_ptr(file) })
}

/// `gmalloc` over [`Alloc::malloc`].
///
/// # Safety
///
/// As [`MallocFn`].
unsafe extern "C" fn malloc_with<A: Alloc>(
    len: usize,
    file: *const c_char,
    line: c_int,
) -> *mut c_void {
    // SAFETY: `MallocFn`'s contract: `file` is NULL or a C string live for the call.
    let file = unsafe { site_file(file) };
    A::malloc(len, file, line).map_or(ptr::null_mut(), |p| p.as_ptr().cast())
}

/// `grealloc` over [`Alloc::realloc`].
///
/// # Safety
///
/// As [`ReallocFn`]: this trampoline's family is `A`'s.
unsafe extern "C" fn realloc_with<A: Alloc>(
    ptr: *mut c_void,
    size: usize,
    file: *const c_char,
    line: c_int,
) -> *mut c_void {
    // SAFETY: as in `malloc_with`.
    let file = unsafe { site_file(file) };
    // SAFETY: `ReallocFn`'s contract: `ptr` is NULL or a live allocation of this
    // family, i.e. one `A` made, which the caller gives up on success, and `size` is
    // non-zero when `ptr` is not NULL.
    let resized = unsafe { A::realloc(NonNull::new(ptr.cast()), size, file, line) };
    resized.map_or(ptr::null_mut(), |p| p.as_ptr().cast())
}

/// `gfree` over [`Alloc::free`]; `free(NULL)` stops here.
///
/// # Safety
///
/// As [`FreeFn`]: this trampoline's family is `A`'s.
unsafe extern "C" fn free_with<A: Alloc>(ptr: *mut c_void) {
    if let Some(ptr) = NonNull::new(ptr.cast()) {
        // SAFETY: `FreeFn`'s contract: a non-NULL `ptr` is a live allocation `A` made,
        // which the caller gives up.
        unsafe { A::free(ptr) }
    }
}

// An `Allocator` is a `git_allocator`, so C reads a value Rust built in place.
const _: () = assert!(
    size_of::<Allocator>() == size_of::<ffi::git_allocator>()
        && align_of::<Allocator>() == align_of::<ffi::git_allocator>()
);

#[cfg(test)]
mod unit_tests {
    use std::alloc::{Layout, alloc, dealloc, realloc};
    use std::cell::RefCell;
    use std::ffi::CString;

    use super::*;

    /// One call that reached an `Alloc` implementation.
    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        Malloc(usize, Option<CString>, i32),
        Realloc(bool, usize, Option<CString>, i32),
        Free,
    }

    thread_local! {
        static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
    }

    fn take_calls() -> Vec<Call> {
        CALLS.with(RefCell::take)
    }

    /// The one request size [`Heap`] refuses.
    const REFUSED: usize = 4242;

    /// A Rust-heap allocator, so the trampolines run under Miri: each block carries its
    /// size in a 16-byte header.
    struct Heap;

    const HEADER: usize = 16;

    fn layout(len: usize) -> Layout {
        Layout::from_size_align(HEADER + len, HEADER).unwrap()
    }

    // SAFETY: blocks come from the global allocator, 16-aligned past their header,
    // fresh per call; `realloc` keeps the prefix and `free` releases with the recorded
    // layout. Thread-safe as the global allocator is.
    unsafe impl Alloc for Heap {
        fn malloc(len: usize, file: Option<&CStr>, line: i32) -> Option<NonNull<u8>> {
            CALLS.with(|c| {
                c.borrow_mut()
                    .push(Call::Malloc(len, file.map(CStr::to_owned), line));
            });
            if len == REFUSED {
                return None;
            }
            // SAFETY: the layout is never zero-sized.
            let base = NonNull::new(unsafe { alloc(layout(len)) })?;
            // SAFETY: the header is the block's first 16 bytes, aligned for a `usize`.
            unsafe { base.cast::<usize>().write(len) };
            // SAFETY: the block spans `HEADER + len` bytes.
            Some(unsafe { base.add(HEADER) })
        }

        unsafe fn realloc(
            ptr: Option<NonNull<u8>>,
            len: usize,
            file: Option<&CStr>,
            line: i32,
        ) -> Option<NonNull<u8>> {
            CALLS.with(|c| {
                let call = Call::Realloc(ptr.is_some(), len, file.map(CStr::to_owned), line);
                c.borrow_mut().push(call);
            });
            let Some(ptr) = ptr else {
                let block = Self::malloc(len, file, line);
                CALLS.with(|c| c.borrow_mut().pop());
                return block;
            };
            if len == REFUSED {
                return None;
            }
            // SAFETY: `ptr` is a live block of this allocator, so its header precedes it.
            let base = unsafe { ptr.sub(HEADER) };
            // SAFETY: as above; the header holds the block's length.
            let old = unsafe { base.cast::<usize>().read() };
            // SAFETY: `base` was allocated with `layout(old)`; the new size is non-zero.
            let base = NonNull::new(unsafe { realloc(base.as_ptr(), layout(old), HEADER + len) })?;
            // SAFETY: as in `malloc`.
            unsafe { base.cast::<usize>().write(len) };
            // SAFETY: as in `malloc`.
            Some(unsafe { base.add(HEADER) })
        }

        unsafe fn free(ptr: NonNull<u8>) {
            CALLS.with(|c| c.borrow_mut().push(Call::Free));
            // SAFETY: `ptr` is a live block of this allocator, so its header precedes it.
            let base = unsafe { ptr.sub(HEADER) };
            // SAFETY: as above; `base` was allocated with `layout(len)`.
            let read = unsafe { base.cast::<usize>().read() };
            // SAFETY: as above.
            unsafe { dealloc(base.as_ptr(), layout(read)) }
        }
    }

    unsafe extern "C" fn c_malloc(_: usize, _: *const c_char, _: c_int) -> *mut c_void {
        ptr::null_mut()
    }

    unsafe extern "C" fn c_realloc(
        _: *mut c_void,
        _: usize,
        _: *const c_char,
        _: c_int,
    ) -> *mut c_void {
        ptr::null_mut()
    }

    unsafe extern "C" fn c_free(_: *mut c_void) {}

    unsafe extern "C" fn other_free(_: *mut c_void) {}

    // Each coercion of a function item may yield a distinct address (Miri models
    // that), so every comparison below is against one stored pointer value.

    #[test]
    #[expect(
        clippy::many_single_char_names,
        reason = "short names for the sides and values the steps compare"
    )]
    fn from_fns_sets_every_member() {
        let (m, r, f): (MallocFn, ReallocFn, FreeFn) = (c_malloc, c_realloc, c_free);
        let a = Allocator::from_fns(m, r, f);
        let h = a.as_ref();
        assert!(ptr::fn_addr_eq(h.malloc().unwrap(), m));
        assert!(ptr::fn_addr_eq(h.realloc().unwrap(), r));
        assert!(ptr::fn_addr_eq(h.free().unwrap(), f));
    }

    #[test]
    #[expect(
        clippy::many_single_char_names,
        reason = "short names for the sides and values the steps compare"
    )]
    fn setters_replace_one_member_each() {
        let (m, r, f, other): (MallocFn, ReallocFn, FreeFn, FreeFn) =
            (c_malloc, c_realloc, c_free, other_free);
        let mut a = Allocator::from_fns(m, r, f);
        let before = a;
        a.as_mut().set_free(other);
        assert!(ptr::fn_addr_eq(a.as_ref().free().unwrap(), other));
        assert!(ptr::fn_addr_eq(a.as_ref().malloc().unwrap(), m));
        let heap = Allocator::new::<Heap>();
        let (hm, hr) = (
            heap.as_ref().malloc().unwrap(),
            heap.as_ref().realloc().unwrap(),
        );
        let mut handle = a.as_mut();
        handle.as_mut().set_malloc(hm);
        handle.set_realloc(hr);
        let h = handle.as_ref();
        assert!(ptr::fn_addr_eq(h.malloc().unwrap(), hm));
        assert!(ptr::fn_addr_eq(h.realloc().unwrap(), hr));
        assert!(ptr::fn_addr_eq(h.free().unwrap(), other));
        // `before` is a copy, untouched.
        let b = before.as_ref();
        assert!(ptr::fn_addr_eq(b.malloc().unwrap(), m));
        assert!(ptr::fn_addr_eq(b.realloc().unwrap(), r));
        assert!(ptr::fn_addr_eq(b.free().unwrap(), f));
    }

    #[test]
    fn trampolines_forward_to_the_implementation() {
        static HEAP: Allocator = Allocator::new::<Heap>();
        let r = HEAP.as_ref();
        let (malloc, realloc, free) =
            (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap());
        take_calls();

        // SAFETY: `file` is a live C string; the block is released below.
        let p = unsafe { malloc(5, c"alloc.c".as_ptr(), 7) };
        assert!(!p.is_null());
        assert_eq!(p as usize % 16, 0);
        // SAFETY: `p` holds 5 writable bytes.
        unsafe {
            p.cast::<u8>()
                .copy_from_nonoverlapping(b"hello".as_ptr(), 5);
        };
        // SAFETY: `p` is a live block of this family, given up; NULL `file`.
        let q = unsafe { realloc(p, 64, ptr::null(), -1) };
        assert!(!q.is_null());
        // SAFETY: the first 5 bytes were carried over.
        let kept = unsafe { core::slice::from_raw_parts(q.cast::<u8>(), 5) };
        assert_eq!(kept, b"hello");
        // SAFETY: a refused resize leaves `q` live.
        assert!(unsafe { realloc(q, REFUSED, ptr::null(), 0) }.is_null());
        // SAFETY: NULL is a fresh allocation; the block is released below.
        let n = unsafe { realloc(ptr::null_mut(), 3, c"x".as_ptr(), 1) };
        assert!(!n.is_null());
        // SAFETY: NULL is a no-op, and `q` and `n` are live blocks given up once.
        unsafe { free(ptr::null_mut()) };
        // SAFETY: as above.
        unsafe { free(q) };
        // SAFETY: as above.
        unsafe { free(n) };
        // SAFETY: as above.
        assert!(unsafe { malloc(REFUSED, ptr::null(), 0) }.is_null());

        assert_eq!(
            take_calls(),
            [
                Call::Malloc(5, Some(c"alloc.c".to_owned()), 7),
                Call::Realloc(true, 64, None, -1),
                Call::Realloc(true, REFUSED, None, 0),
                Call::Realloc(false, 3, Some(c"x".to_owned()), 1),
                Call::Free,
                Call::Free,
                Call::Malloc(REFUSED, None, 0),
            ]
        );
    }

    /// A zero-byte request is an allocation like any other: distinct, and released by
    /// `free`, through `malloc` and through `realloc(NULL, ..)` alike.
    #[test]
    fn zero_byte_requests_are_allocations() {
        static HEAP: Allocator = Allocator::new::<Heap>();
        let r = HEAP.as_ref();
        let (malloc, realloc, free) =
            (r.malloc().unwrap(), r.realloc().unwrap(), r.free().unwrap());
        take_calls();
        // SAFETY: NULL `file`; both blocks are released below.
        let malloc2 = unsafe { malloc(0, ptr::null(), 0) };
        // SAFETY: as above.
        let (a, b) = unsafe { (malloc2, realloc(ptr::null_mut(), 0, ptr::null(), 0)) };
        assert!(!a.is_null() && !b.is_null() && a != b);
        // A zero-byte block grows like any other, keeping its (empty) prefix.
        // SAFETY: `a` is a live block of this family, given up on success.
        let grown = unsafe { realloc(a, 24, c"x".as_ptr(), 1) };
        assert!(!grown.is_null());
        // SAFETY: `grown` holds 24 writable bytes.
        unsafe { grown.cast::<u8>().write_bytes(0xAB, 24) };
        // SAFETY: two live blocks of this family, each given up once.
        unsafe { free(grown) };
        // SAFETY: as above.
        unsafe { free(b) };
        assert_eq!(
            take_calls(),
            [
                Call::Malloc(0, None, 0),
                Call::Realloc(false, 0, None, 0),
                Call::Realloc(true, 24, Some(c"x".to_owned()), 1),
                Call::Free,
                Call::Free,
            ]
        );
    }

    /// An implementation that panics: libgit2 would call the trampoline from C, so the
    /// panic cannot unwind and aborts the process instead of corrupting a C frame.
    struct Panicking;

    // SAFETY: never returns an allocation, so nothing is ever live.
    unsafe impl Alloc for Panicking {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            panic!("refusing {len} bytes");
        }

        unsafe fn realloc(
            _: Option<NonNull<u8>>,
            _: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            None
        }

        unsafe fn free(_: NonNull<u8>) {}
    }

    const CHILD: &str = "CRUSTIFY_ALLOC_UNIT_CHILD";

    #[test]
    #[cfg_attr(miri, ignore = "spawns a process")]
    #[cfg_attr(bsan, ignore = "the child's abort path spins under BorrowSanitizer")]
    fn panic_in_the_implementation_aborts() {
        // The test's path within the binary: `module_path!` less the crate name.
        let path = concat!(module_path!(), "::panic_in_the_implementation_aborts");
        let name = path.split_once("::").map_or(path, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            static PANICKING: Allocator = Allocator::new::<Panicking>();
            let malloc = PANICKING.as_ref().malloc().unwrap();
            // SAFETY: NULL `file`; the implementation panics before allocating.
            let _ = unsafe { malloc(1, ptr::null(), 0) };
            unreachable!("the trampoline must not return from a panic");
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{stderr}");
        assert!(stderr.contains("refusing 1 bytes"), "{stderr}");
        assert!(stderr.contains("cannot unwind"), "{stderr}");
        assert!(!stderr.contains("must not return"), "{stderr}");
    }

    #[test]
    fn debug_lists_member_addresses() {
        let f: FreeFn = c_free;
        let a = Allocator::from_fns(c_malloc, c_realloc, f);
        let shown = format!("{a:?}");
        assert!(shown.starts_with("Allocator { malloc: 0x"), "{shown}");
        assert_eq!(shown, format!("{:?}", a.as_ref()));
        assert!(
            shown.ends_with(&format!("free: {:p} }}", f as *const ())),
            "{shown}"
        );

        // A member C left NULL shows as a null address.
        let mut raw = ffi::git_allocator::default();
        // SAFETY: `raw` is a live `git_allocator` for the handle's lifetime.
        let r = unsafe { AllocatorRef::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(
            format!("{r:?}"),
            "Allocator { malloc: 0x0, realloc: 0x0, free: 0x0 }"
        );
        assert!(r.malloc().is_none() && r.realloc().is_none() && r.free().is_none());
    }

    #[test]
    #[expect(
        clippy::items_after_statements,
        reason = "each helper item sits beside the steps that use it"
    )]
    fn value_is_copy_send_and_sync() {
        fn shared<T: Copy + Send + Sync>() {}
        fn handle<T: Send + Sync>() {}
        shared::<Allocator>();
        handle::<AllocatorRef<'static>>();
        handle::<AllocatorMut<'static>>();

        static HEAP: Allocator = Allocator::new::<Heap>();
        let got = std::thread::spawn(|| HEAP.as_ref().free().map(|f| f as *const () as usize))
            .join()
            .unwrap();
        assert_eq!(got, HEAP.as_ref().free().map(|f| f as *const () as usize));
    }
}
