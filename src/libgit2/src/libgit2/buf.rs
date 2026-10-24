//! Safe wrappers over `include/git2/buffer.h`.
//!
//! A [`Buf`] is a `git_buf`: the struct libgit2 returns data through, `size` bytes at
//! `ptr`, NUL-terminated, in libgit2's allocator. libgit2 never allocates the struct
//! itself: callers hold it by value, pass it to a routine that fills it, and release what
//! it points at with `git_buf_dispose`. Rust holds it the same way, as a [`BufVal`], which
//! disposes it on drop.
//!
//! The header calls `reserved` "unused"; it is not. libgit2 fills a buffer from an internal
//! `git_str` (`git_buf_fromstr`), storing the string's allocation size there, and reads it
//! back when a buffer is passed in again (`git_buf_sanitize`, `git_buf_tostr`):
//!
//! - `reserved > 0`: `ptr` is a live buffer of at least `reserved` bytes from libgit2's allocator,
//!   owned by the struct, with `size < reserved` and `ptr[size] == 0`. libgit2 reuses it for the
//!   next result, reallocating it with `git__realloc` as needed.
//! - `reserved == 0`: the struct owns nothing. `ptr` is NULL (`GIT_BUF_INIT`) or libgit2's static
//!   empty string, which an empty result is filled with without allocating, and which libgit2
//!   replaces without freeing when the buffer is passed in.
//!
//! Every handle upholds that invariant, which the safe setters keep and a crate-internal
//! `from_ptr` over a C-owned struct, or [`Buf::from_raw`], must vouch for; libgit2's
//! output routines keep it too, so the crate fills a buffer by passing
//! `as_mut().as_mut_ptr()` to one. A buffer with `reserved == 0` reads as empty whatever
//! `ptr` and `size` hold, and is never passed to `git_buf_dispose`: the routine frees
//! every `ptr` but the static empty string, so it would free a NULL `ptr` through
//! `git__free`, which reads the allocator a concurrent first `git_libgit2_init` writes.
//! An owned buffer was allocated by libgit2's allocator, which initialised the library
//! first.
//!
//! `ptr` moves in and out as one of the owners of libgit2's allocator in
//! [`raw`](crate::libgit2::src::libgit2::odb): an [`OdbDataStr`] for a string
//! ([`set_str`](BufMut::set_str) / [`take_str`](BufMut::take_str)), or an
//! [`OdbDataVec<u8>`] for contents that may hold NUL bytes
//! ([`take_bytes`](BufMut::take_bytes)), such as a pack or a binary patch.
//!
//! There is no public `Buf` value outside an owner: one holding an allocation would leak
//! it when dropped, so the constructors return a [`BufVal`], and `Buf` is not
//! `CZeroable`. [`CVal::into_inner`] still surrenders the bare value, as `mem::forget`
//! surrenders a `Vec`, and [`Buf::into_raw`] hands it to C.

use core::ffi::{CStr, c_char};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use core::str::Utf8Error;

use ffibox::{CDispose, CVal, define_ctype};

use crate::ffi;
use crate::libgit2::src::libgit2::odb::{OdbDataArrayFree, OdbDataStr, OdbDataVec};

define_ctype!(
    /// A buffer libgit2 returns data through (`git_buf`): [`len`](BufRef::len) bytes,
    /// NUL-terminated, in one allocation of libgit2's that the struct owns, or nothing.
    ///
    /// Held by value inside a [`BufVal`], which [`new`](Self::new) and
    /// [`from_bytes`](Self::from_bytes) build; read and write it through the owner's
    /// [`as_ref`](CVal::as_ref) / [`as_mut`](CVal::as_mut) handles. Not `Clone`: it owns
    /// its allocation, so a copy is [`BufRef::to_buf`], which allocates.
    ///
    /// Wraps: git_buf
    Buf,
    BufRef,
    BufMut,
    ffi::git_buf
);

// SAFETY: a `Buf` owns its allocation exclusively, or points at nothing it owns (NULL,
// or libgit2's static empty string, which nothing writes). The allocation is
// libgit2-allocator storage, which `git__free` releases on any thread (as
// `OdbDataVec<u8>`, which is `Send`), and no libgit2 routine keeps thread-local state
// about it; moving the struct moves sole ownership of it.
unsafe impl Send for Buf {}

// SAFETY: `BufMut` is an exclusive borrow of such a value, as `&mut Buf`.
unsafe impl Send for BufMut<'_> {}

// SAFETY: shared access only reads `ptr`, `reserved`, `size` and the bytes they address:
// every write goes through `BufMut`, which excludes other handles, and no libgit2 routine
// takes a `const git_buf *` to write through.
unsafe impl Sync for Buf {}

// SAFETY: as above; `BufRef` is a shared borrow, as `&Buf`.
unsafe impl Send for BufRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for BufRef<'_> {}

// SAFETY: `&BufMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for BufMut<'_> {}

/// The empty buffer, `GIT_BUF_INIT`.
const INIT: ffi::git_buf = ffi::git_buf {
    ptr: ptr::null_mut(),
    reserved: 0,
    size: 0,
};

/// Releases a buffer's allocation through `git_buf_dispose`, which hands `ptr` to
/// `git__free` and leaves the struct, Rust's storage, alone.
///
/// A buffer owning nothing (`reserved == 0`) is not passed to C: there is nothing to
/// free, and the call would be `git__free(NULL)` for `GIT_BUF_INIT`, which reads
/// `git__allocator` without synchronisation against the first `git_libgit2_init` another
/// thread may be running. Nothing orders an unallocated buffer after that initialisation.
///
/// Wraps: `git_buf_dispose`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct BufDispose;

// SAFETY: `git_buf_dispose` frees `ptr` exactly once and never the struct. It accepts
// every state the safe API reaches with `reserved > 0`: an allocation of libgit2's
// allocator that the struct owns (whose allocation initialised the library, so the
// allocator it reads is settled), never the static empty string. A buffer with
// `reserved == 0` owns nothing and is released without C.
unsafe impl CDispose<Buf> for BufDispose {
    #[inline]
    unsafe fn c_dispose(&self, ptr: NonNull<Buf>) {
        let raw: *mut ffi::git_buf = ptr.as_ptr().cast();
        // SAFETY: per `c_dispose`'s contract `raw` addresses a live `git_buf`.
        let reserved_ptr = unsafe { addr_of!((*raw).reserved) };
        // SAFETY: as above.
        let reserved = unsafe { reserved_ptr.read() };
        if reserved == 0 {
            return;
        }
        // SAFETY: `raw` is live and, per the handles' invariant, owns `ptr`, an
        // allocation of libgit2's allocator; the caller gives the value up, so it is
        // released once.
        unsafe { ffi::git_buf_dispose(raw) }
    }
}

/// An owned [`Buf`], held inline; its allocation is released by `git_buf_dispose` on
/// drop.
pub type BufVal = CVal<Buf, BufDispose>;

impl Buf {
    /// An empty buffer owning nothing, as `git_buf buf = GIT_BUF_INIT`: the storage
    /// libgit2 fills a result into.
    pub fn new() -> BufVal {
        // SAFETY: an empty value owns nothing, which `git_buf_dispose` accepts.
        unsafe { CVal::new(Self(INIT, PhantomData)) }
    }

    /// A buffer holding a copy of `bytes`, NUL-terminated, in a fresh allocation of
    /// libgit2's allocator, as libgit2 hands a result out; empty and unallocated for no
    /// bytes.
    ///
    /// `None` when the allocation fails, or libgit2 cannot be initialised.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<BufVal> {
        if bytes.is_empty() {
            return Some(Self::new());
        }
        // A slice spans at most `isize::MAX` bytes, so the terminator's byte fits.
        let reserved = bytes.len() + 1;
        let mut storage = OdbDataArrayFree::new_uninit::<u8>(reserved)?;
        let dst = storage.as_mut_slice();
        // SAFETY: `dst` is a fresh, distinct allocation of `reserved` bytes, so the copy
        // of `bytes.len()` bytes fits and does not overlap `bytes`; `MaybeUninit<u8>` has
        // `u8`'s layout.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), dst.as_mut_ptr().cast::<u8>(), bytes.len());
        }
        dst[bytes.len()] = MaybeUninit::new(0);
        let (data, _) = storage.into_raw_parts();
        let raw = ffi::git_buf {
            ptr: data.cast(),
            reserved,
            size: bytes.len(),
        };
        // SAFETY: `raw` is the sole owner of `data`, a fresh allocation of `reserved`
        // bytes from libgit2's allocator, holding `bytes.len()` initialised bytes and the
        // NUL after them, which `git_buf_dispose` releases.
        Some(unsafe { CVal::new(Self(raw, PhantomData)) })
    }

    /// Adopts a `git_buf` C code filled, as libgit2 hands one out: what it owns becomes
    /// the result's, released by `git_buf_dispose` on drop.
    ///
    /// A buffer with `reserved == 0` owns nothing and reads as empty, whatever `ptr` and
    /// `size` hold.
    ///
    /// # Safety
    ///
    /// When `raw.reserved > 0`, `raw.ptr` must be a live allocation of at least
    /// `raw.reserved` bytes from libgit2's current allocator, which `raw` owns and the
    /// caller gives up, with `raw.size < raw.reserved`, its first `raw.size` bytes
    /// initialised and a NUL at `raw.ptr[raw.size]`: as libgit2's routines leave a buffer
    /// they filled. Nothing else may read or write the allocation while the result lives.
    /// With `raw.reserved == 0` there is no requirement: `raw.ptr` and `raw.size` are
    /// never read through.
    pub unsafe fn from_raw(raw: ffi::git_buf) -> BufVal {
        // SAFETY: the caller gives up `raw`, which owns what it points at (this
        // function's contract).
        unsafe { CVal::new(Self(raw, PhantomData)) }
    }

    /// The raw `git_buf`, ownership included: C code releases it with `git_buf_dispose`
    /// or passes it to a libgit2 routine to fill again.
    #[must_use]
    pub fn into_raw(self) -> ffi::git_buf {
        self.0
    }
}

impl<'a> BufRef<'a> {
    /// The `git_buf` fields, `reserved` first; `ptr` is NULL when the buffer owns nothing.
    fn parts(&self) -> (usize, *const u8, usize) {
        let buf = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to a live `git_buf`; any
        // `size_t` is a valid `usize`.
        let reserved_ptr = unsafe { addr_of!((*buf).reserved) };
        // SAFETY: as above.
        let read2 = unsafe { reserved_ptr.read() };
        // SAFETY: as above.
        let ptr_ptr = unsafe { addr_of!((*buf).ptr) };
        // SAFETY: as above.
        let read3 = unsafe { ptr_ptr.read() };
        // SAFETY: as above.
        let size_ptr = unsafe { addr_of!((*buf).size) };
        // SAFETY: as above.
        let (reserved, data, size) = unsafe { (read2, read3, size_ptr.read()) };
        if reserved == 0 {
            (0, ptr::null(), 0)
        } else {
            (reserved, data.cast_const().cast(), size)
        }
    }

    /// The number of bytes held, the terminating NUL excluded; 0 for a buffer owning
    /// nothing, whatever `size` says, as [`as_bytes`](Self::as_bytes) reads it.
    ///
    /// Field: `git_buf.size`
    #[must_use]
    pub fn len(&self) -> usize {
        self.parts().2
    }

    /// Whether the buffer holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The size of the allocation the buffer owns, the room for the terminating NUL
    /// included, which libgit2 reuses before reallocating; 0 when it owns none.
    ///
    /// Field: `git_buf.reserved`
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.parts().0
    }

    /// The contents, borrowed in place for as long as the buffer is, NUL bytes included;
    /// empty for a buffer owning nothing.
    ///
    /// Field: `git_buf.ptr`
    #[must_use]
    pub fn as_bytes(&self) -> &'a [u8] {
        match self.parts() {
            (0, _, _) => &[],
            // SAFETY: per the handles' invariant an owned buffer holds `size` initialised
            // bytes at `ptr` (followed by its NUL), owned by the struct, so they live as
            // long as it does, past `'a`; nothing writes them while it is shared.
            (_, data, size) => unsafe { core::slice::from_raw_parts(data, size) },
        }
    }

    /// The contents up to the first NUL byte, as C code reads `ptr`: all of them unless
    /// they hold a NUL of their own. Empty for a buffer owning nothing.
    #[must_use]
    pub fn as_c_str(&self) -> &'a CStr {
        match self.parts() {
            (0, _, _) => c"",
            (_, data, size) => {
                // SAFETY: as in `as_bytes`, with the terminator at `ptr[size]` too.
                let bytes = unsafe { core::slice::from_raw_parts(data, size + 1) };
                // The last byte is a NUL, so there always is one.
                CStr::from_bytes_until_nul(bytes).unwrap_or(c"")
            }
        }
    }

    /// The contents as UTF-8; an error when they are not.
    ///
    /// # Errors
    ///
    /// Fails with the [`Utf8Error`] of the first invalid byte.
    pub fn to_str(&self) -> Result<&'a str, Utf8Error> {
        core::str::from_utf8(self.as_bytes())
    }

    /// A copy of the buffer, its contents in a fresh allocation of libgit2's allocator;
    /// libgit2 has no copy routine. Empty and unallocated for an empty buffer.
    ///
    /// `None` when the allocation fails, or libgit2 cannot be initialised.
    ///
    /// The owner itself has no `Clone`: a byte copy would dispose the allocation twice.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::src::libgit2::buf::{Buf, BufVal};
    ///
    /// let buf = Buf::new();
    /// let copy: BufVal = buf.clone();
    /// ```
    #[must_use]
    pub fn to_buf(&self) -> Option<BufVal> {
        Buf::from_bytes(self.as_bytes())
    }
}

impl fmt::Debug for BufRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Buf(\"{}\")", self.as_bytes().escape_ascii())
    }
}

impl fmt::Debug for BufMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl BufMut<'_> {
    /// Moves `raw` in, releasing what the buffer held before.
    ///
    /// # Safety
    ///
    /// `raw` keeps the handles' invariant, and its allocation, if any, is handed over.
    unsafe fn replace_raw(&mut self, raw: ffi::git_buf) {
        drop(self.take());
        // SAFETY: a write through the exclusive handle's pointer to a live `git_buf`,
        // whose previous allocation `take` released; `raw` keeps the invariant, per this
        // function's contract.
        unsafe { self.as_mut_ptr().write(raw) }
    }

    /// Replaces the contents with `value`'s, releasing the ones held before.
    ///
    /// Field: `git_buf.ptr`
    /// Field: `git_buf.reserved`
    /// Field: `git_buf.size`
    pub fn set(&mut self, value: BufVal) {
        let raw = value.into_inner().into_raw();
        // SAFETY: `raw` comes out of a `BufVal`, so it keeps the invariant, and its owner
        // gave the allocation up.
        unsafe { self.replace_raw(raw) }
    }

    /// Takes the contents out, leaving the buffer empty and owning nothing
    /// (`GIT_BUF_INIT`): what `git_buf_dispose` would release, owned.
    ///
    /// Field: `git_buf.ptr`
    /// Field: `git_buf.reserved`
    /// Field: `git_buf.size`
    pub fn take(&mut self) -> BufVal {
        let buf = self.as_mut_ptr();
        // SAFETY: reads the struct, then writes the empty buffer, through the exclusive
        // handle's pointer to a live `git_buf`.
        let raw = unsafe { ptr::replace(buf, INIT) };
        // SAFETY: `raw` was the handle's, so it keeps the invariant, and what it owned is
        // handed over whole, since the buffer no longer refers to it.
        unsafe { Buf::from_raw(raw) }
    }

    /// Replaces the contents with a copy of `bytes`, as `git_buf_set` does: in place when
    /// the allocation the buffer owns has room for them and their NUL, in a fresh one
    /// otherwise, releasing the old.
    ///
    /// `None` when that allocation fails, or libgit2 cannot be initialised; the buffer is
    /// left as it was.
    ///
    /// Field: `git_buf.ptr`
    /// Field: `git_buf.size`
    pub fn set_bytes(&mut self, bytes: &[u8]) -> Option<()> {
        let (reserved, data, _) = self.as_ref().parts();
        if bytes.len() < reserved {
            let data = data.cast_mut();
            // SAFETY: per the handles' invariant `data` is an owned allocation of
            // `reserved` bytes, which this exclusive handle may write; `bytes.len() + 1`
            // of them fit. `ptr::copy` tolerates `bytes` overlapping it.
            unsafe { ptr::copy(bytes.as_ptr(), data, bytes.len()) };
            // SAFETY: as above.
            let value2 = unsafe { data.add(bytes.len()) };
            // SAFETY: as above.
            unsafe { value2.write(0) };
            // SAFETY: as above.
            let value2 = unsafe { addr_of_mut!((*self.as_mut_ptr()).size) };
            // SAFETY: as above.
            unsafe { value2.write(bytes.len()) };
            return Some(());
        }
        if bytes.is_empty() {
            // An empty buffer owning nothing already holds no bytes.
            return Some(());
        }
        self.set(Buf::from_bytes(bytes)?);
        Some(())
    }

    /// Replaces the contents with `s`, adopting its allocation without a copy; the old
    /// contents are released.
    ///
    /// Field: `git_buf.ptr`
    pub fn set_str(&mut self, s: OdbDataStr) {
        let size = s.len();
        let data = s.into_raw();
        let raw = ffi::git_buf {
            ptr: data,
            reserved: size + 1,
            size,
        };
        // SAFETY: `data` is a NUL-terminated string of `size` bytes in libgit2's
        // allocator, so its allocation holds at least `size + 1` bytes, with the NUL at
        // `size`; `into_raw` handed it over.
        unsafe { self.replace_raw(raw) }
    }

    /// Takes the contents out as a string, leaving the buffer empty and owning nothing:
    /// what `git_buf_dispose` would release, owned.
    ///
    /// `None` when the buffer owns no allocation, the usual shape of an empty result,
    /// which libgit2 fills without allocating. The string reads up to the first NUL byte;
    /// contents past one are not represented, though the whole allocation is still
    /// released. [`take_bytes`](Self::take_bytes) keeps them.
    ///
    /// Field: `git_buf.ptr`
    pub fn take_str(&mut self) -> Option<OdbDataStr> {
        let raw = self.take().into_inner().into_raw();
        if raw.reserved == 0 {
            return None;
        }
        // SAFETY: per the handles' invariant `reserved > 0` means `ptr` is a live
        // allocation of libgit2's, NUL-terminated at `size`, that the buffer owned and
        // `take` handed over; `OdbDataStrFree` releases it with `git__free`.
        unsafe { OdbDataStr::from_raw(raw.ptr) }
    }

    /// Takes the contents out as bytes, NUL bytes included and the terminator excluded,
    /// leaving the buffer empty and owning nothing: what `git_buf_dispose` would release,
    /// owned. Empty and unallocated when the buffer owns no allocation.
    ///
    /// Field: `git_buf.ptr`
    pub fn take_bytes(&mut self) -> OdbDataVec<u8> {
        let raw = self.take().into_inner().into_raw();
        if raw.reserved == 0 {
            return OdbDataVec::empty();
        }
        // SAFETY: per the handles' invariant `reserved > 0` means `ptr` is a live
        // allocation of libgit2's holding at least `size` initialised bytes, which the
        // buffer owned and `take` handed over. `OdbDataArrayFree` frees with `git__free`,
        // which ignores the length, so the terminator and spare capacity are released
        // with it.
        let bytes = unsafe { OdbDataVec::from_raw_parts(raw.ptr.cast::<u8>(), raw.size) };
        bytes.unwrap_or_default()
    }
}

// A `Buf` is a `git_buf`, so C reads and writes a value Rust holds in place, and the
// owner adds nothing to it.
const _: () = assert!(
    size_of::<Buf>() == size_of::<ffi::git_buf>()
        && align_of::<Buf>() == align_of::<ffi::git_buf>()
        && size_of::<BufVal>() == size_of::<ffi::git_buf>()
);

// `ptr` is a `char *`, which the owners of libgit2's allocator hold as bytes.
const _: () = assert!(size_of::<c_char>() == size_of::<u8>());

#[cfg(test)]
mod unit_tests {
    use core::ffi::c_void;
    use core::mem::size_of;
    use std::process::Command;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering::SeqCst;

    use super::*;
    use crate::libgit2::src::libgit2::odb::OdbDataStrFree;
    use crate::libgit2::src::util::alloc::{Alloc, Allocator};
    use crate::libgit2::src::util::errors::runtime_pinned;

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

    fn install_refusing_allocator() {
        let allocator = Allocator::new::<Refusing>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// The raw struct a handle addresses, read through its pointer.
    fn raw(buf: BufRef<'_>) -> ffi::git_buf {
        // SAFETY: a read through the shared handle's pointer to a live `git_buf`.
        unsafe { buf.as_ptr().read() }
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<Buf>(), size_of::<ffi::git_buf>());
        assert_eq!(size_of::<BufVal>(), size_of::<ffi::git_buf>());
        assert_eq!(size_of::<BufRef<'_>>(), size_of::<*const ffi::git_buf>());
        assert_eq!(size_of::<BufMut<'_>>(), size_of::<*const ffi::git_buf>());
        assert_eq!(size_of::<BufDispose>(), 0);
    }

    /// The empty buffer's accessors over a bare value, without libgit2, so Miri runs
    /// them; nothing here allocates or disposes.
    #[test]
    fn empty_buffer_accessors_in_rust_storage() {
        let mut bare = Buf::new().into_inner();
        let mut m = bare.as_mut();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().is_empty());
        assert_eq!(m.as_ref().capacity(), 0);
        assert_eq!(m.as_ref().as_bytes(), b"");
        assert_eq!(m.as_ref().as_c_str(), c"");
        assert_eq!(m.as_ref().to_str(), Ok(""));
        assert_eq!(m.set_bytes(b""), Some(()));
        assert!(m.take_str().is_none());
        assert!(m.take_bytes().is_empty());
        let taken = m.take();
        assert_eq!(raw(taken.as_ref()).reserved, 0);
        m.set(taken);
        m.set(Buf::from_bytes(b"").unwrap());
        assert_eq!(format!("{m:?}"), r#"Buf("")"#);
        assert_eq!(format!("{:?}", bare.as_ref()), r#"Buf("")"#);
        let raw = bare.into_raw();
        assert!(raw.ptr.is_null() && raw.reserved == 0 && raw.size == 0);
    }

    /// A buffer with `reserved == 0` owns nothing and reads as empty, whatever `ptr`
    /// and `size` hold, and the dispose policy never calls libgit2: `git_buf_dispose`
    /// would `git__free` this `ptr` (Miri aborts on any foreign call, and the plain
    /// build would crash, so this proves the drop makes none).
    #[test]
    fn unowned_buffer_reads_empty_and_disposes_without_c() {
        let stray = ffi::git_buf {
            ptr: NonNull::<c_char>::dangling().as_ptr(),
            reserved: 0,
            size: 5,
        };
        // SAFETY: `reserved == 0`, for which `from_raw` has no requirement.
        let mut buf = unsafe { Buf::from_raw(stray) };
        let r = buf.as_ref();
        assert_eq!((r.len(), r.capacity(), r.is_empty()), (0, 0, true));
        assert_eq!(r.as_bytes(), b"");
        assert_eq!(r.as_c_str(), c"");
        assert_eq!(format!("{r:?}"), r#"Buf("")"#);
        assert!(r.to_buf().unwrap().as_ref().is_empty());
        let value = NonNull::new(buf.as_mut().as_mut_ptr().cast::<Buf>()).unwrap();
        // SAFETY: `value` addresses the live value `buf` holds; the policy's contract
        // accepts a buffer that owns nothing.
        unsafe { BufDispose.c_dispose(value) };
        assert_eq!(raw(buf.as_ref()).size, 5, "left as it was");
        assert!(buf.as_mut().take_str().is_none());
        assert_eq!(raw(buf.as_ref()).size, 0, "taken, it is GIT_BUF_INIT");
        // SAFETY: as above.
        drop(unsafe { Buf::from_raw(stray) });
    }

    /// The handles' projections and in-place edits over a buffer Rust allocated, so
    /// Miri checks them; the buffer leaves through `into_raw`, so nothing reaches
    /// libgit2's free.
    #[test]
    fn handles_over_rust_storage() {
        let mut storage = *b"hello\0\xff\xff";
        let base = storage.as_mut_ptr();
        let mut raw_buf = ffi::git_buf {
            ptr: base.cast(),
            reserved: 8,
            size: 5,
        };
        // SAFETY: `raw_buf` is live for the handle's use and owns 8 bytes the test keeps
        // alive, NUL-terminated at 5; no other handle reaches it meanwhile. It is never
        // released through the handle: the take below hands it back raw.
        let mut m = unsafe { BufMut::from_ptr(&raw mut raw_buf) }.unwrap();
        let r = m.as_ref();
        assert_eq!((r.len(), r.capacity()), (5, 8));
        assert_eq!(r.as_bytes(), b"hello");
        assert_eq!(r.as_c_str(), c"hello");
        assert_eq!(r.to_str(), Ok("hello"));
        assert_eq!(format!("{m:?}"), r#"Buf("hello")"#);

        // Fits with its NUL: written in place.
        assert_eq!(m.set_bytes(b"a\0b\x01c\x7f\n"), Some(()));
        let r = m.as_ref();
        assert_eq!(r.as_bytes(), b"a\0b\x01c\x7f\n");
        assert_eq!(r.as_c_str(), c"a");
        assert_eq!(r.capacity(), 8);
        assert_eq!(format!("{r:?}"), r#"Buf("a\x00b\x01c\x7f\n")"#);
        assert!(m.as_ref().to_str().is_ok());
        assert_eq!(m.set_bytes(b""), Some(()));
        assert_eq!((m.as_ref().len(), m.as_ref().as_c_str()), (0, c""));
        assert_eq!(m.set_bytes(b"\xc3\x28"), Some(()));
        assert!(m.as_ref().to_str().is_err());

        let back = m.take().into_inner().into_raw();
        assert_eq!(
            (back.ptr.cast::<u8>(), back.reserved, back.size),
            (base, 8, 2)
        );
        assert!(raw_buf.ptr.is_null() && raw_buf.reserved == 0 && raw_buf.size == 0);
        assert_eq!(&storage[..3], b"\xc3\x28\0");
    }

    /// Every constructor, copy, move and replacement releases each allocation once
    /// (`LSan`, in the asan variant, flags a leak, `ASan` a double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owned_buffers_copy_move_and_release() {
        let mut buf = Buf::from_bytes(b"refs/heads/main").unwrap();
        let r = buf.as_ref();
        assert_eq!((r.len(), r.capacity()), (15, 16));
        assert_eq!(r.to_str(), Ok("refs/heads/main"));
        let first = raw(r).ptr;

        let copy = r.to_buf().unwrap();
        assert_ne!(
            raw(copy.as_ref()).ptr,
            first,
            "a copy has an allocation of its own"
        );
        assert_eq!(copy.as_ref().as_bytes(), b"refs/heads/main");

        // Shorter: in place. Longer than the allocation: a fresh one.
        buf.as_mut().set_bytes(b"main").unwrap();
        assert_eq!(
            (raw(buf.as_ref()).ptr, buf.as_ref().capacity()),
            (first, 16)
        );
        assert_eq!(buf.as_ref().as_c_str(), c"main");
        buf.as_mut().set_bytes(&[b'x'; 16]).unwrap();
        assert_eq!(buf.as_ref().capacity(), 17);
        assert_eq!(buf.as_ref().as_bytes(), [b'x'; 16]);

        buf.as_mut().set(copy);
        assert_eq!(buf.as_ref().as_bytes(), b"refs/heads/main");
        let moved = buf.as_mut().take();
        assert!(buf.as_ref().is_empty() && buf.as_ref().capacity() == 0);
        assert_eq!(moved.as_ref().as_bytes(), b"refs/heads/main");

        buf.as_mut()
            .set_str(OdbDataStrFree::dup(c"origin").unwrap());
        assert_eq!((buf.as_ref().len(), buf.as_ref().capacity()), (6, 7));
        assert_eq!(buf.as_ref().as_c_str(), c"origin");
        let s = buf.as_mut().take_str().unwrap();
        assert_eq!(s.as_c_str(), c"origin");
        assert!(buf.as_mut().take_str().is_none(), "nothing left to take");

        buf.as_mut().set(Buf::from_bytes(b"pa\0ck").unwrap());
        let bytes = buf.as_mut().take_bytes();
        assert_eq!(bytes.as_slice(), b"pa\0ck");
        assert!(buf.as_mut().take_bytes().is_empty());

        // An owned, empty buffer: `take_str` still hands the allocation over.
        buf.as_mut().set(Buf::from_bytes(b"x").unwrap());
        buf.as_mut().set_bytes(b"").unwrap();
        assert_eq!((buf.as_ref().len(), buf.as_ref().capacity()), (0, 2));
        assert_eq!(buf.as_mut().take_str().unwrap().as_c_str(), c"");
        buf.as_mut().set(Buf::from_bytes(b"y").unwrap());
        buf.as_mut().set_bytes(b"").unwrap();
        let empty = buf.as_mut().take_bytes();
        assert!(empty.is_empty() && !empty.as_ptr().is_null());

        // The bare value round-trips through C's struct.
        let bare = moved.into_inner().into_raw();
        // SAFETY: `bare` came out of a `BufVal`, so it owns its NUL-terminated
        // allocation of libgit2's.
        let back = unsafe { Buf::from_raw(bare) };
        assert_eq!(back.as_ref().as_bytes(), b"refs/heads/main");
        drop((back, s, bytes, empty, buf));
    }

    /// `from_raw` adopts what libgit2 fills: an owned string from
    /// `GIT_OPT_GET_USER_AGENT`, reused and reallocated when the buffer is passed in
    /// again, and the unallocated static empty string from an empty
    /// `GIT_OPT_GET_TEMPLATE_PATH`, which reads as empty and is never freed.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopts_what_libgit2_fills() {
        isolated(
            concat!(module_path!(), "::adopts_what_libgit2_fills"),
            || {
                assert!(runtime_pinned());
                let opt = |opt: u32, s: *const c_char| {
                    // SAFETY: both options copy the string, or restore the default for NULL.
                    assert_eq!(unsafe { ffi::git_libgit2_opts(opt.cast_signed(), s) }, 0);
                };
                let get = |opt: u32, buf: &mut BufVal| {
                    let mut raw = buf.as_mut().take().into_inner().into_raw();
                    // SAFETY: both options take a `git_buf *` that is zeroed or owns a
                    // libgit2 allocation, which they reuse or replace.
                    let rc = unsafe { ffi::git_libgit2_opts(opt.cast_signed(), &raw mut raw) };
                    assert_eq!(rc, 0);
                    // SAFETY: `raw` is as libgit2 left it.
                    buf.as_mut().set(unsafe { Buf::from_raw(raw) });
                };

                opt(ffi::GIT_OPT_SET_USER_AGENT, c"crustify/1.0".as_ptr());
                let mut buf = Buf::new();
                get(ffi::GIT_OPT_GET_USER_AGENT, &mut buf);
                assert_eq!(buf.as_ref().to_str(), Ok("crustify/1.0"));
                assert!(buf.as_ref().capacity() > buf.as_ref().len());

                // Passed in again, an allocation is reused (and grown), whoever made it.
                let mut ours = Buf::from_bytes(b"0123456789abcdef0123").unwrap();
                let before = raw(ours.as_ref()).ptr;
                get(ffi::GIT_OPT_GET_USER_AGENT, &mut ours);
                assert_eq!(ours.as_ref().to_str(), Ok("crustify/1.0"));
                assert_eq!(raw(ours.as_ref()).ptr, before, "fits: reused in place");
                opt(
                    ffi::GIT_OPT_SET_USER_AGENT,
                    c"a much longer user agent than before".as_ptr(),
                );
                get(ffi::GIT_OPT_GET_USER_AGENT, &mut ours);
                assert_eq!(
                    ours.as_ref().as_c_str(),
                    c"a much longer user agent than before"
                );

                opt(ffi::GIT_OPT_SET_TEMPLATE_PATH, c"".as_ptr());
                let mut empty = Buf::new();
                get(ffi::GIT_OPT_GET_TEMPLATE_PATH, &mut empty);
                let filled = raw(empty.as_ref());
                assert!(
                    !filled.ptr.is_null() && filled.reserved == 0,
                    "the static empty string"
                );
                assert_eq!((empty.as_ref().len(), empty.as_ref().capacity()), (0, 0));
                assert_eq!(empty.as_ref().as_c_str(), c"");
                assert!(empty.as_mut().take_str().is_none());
                get(ffi::GIT_OPT_GET_TEMPLATE_PATH, &mut empty);
                drop(empty);

                // An owned buffer passed in for an empty result keeps its allocation.
                get(ffi::GIT_OPT_GET_TEMPLATE_PATH, &mut buf);
                assert!(buf.as_ref().is_empty() && buf.as_ref().capacity() > 0);
                assert_eq!(buf.as_ref().as_c_str(), c"");

                opt(ffi::GIT_OPT_SET_TEMPLATE_PATH, ptr::null());
                opt(ffi::GIT_OPT_SET_USER_AGENT, ptr::null());
            },
        );
    }

    /// A failed allocation is `None` from both copying constructors and from a growing
    /// `set_bytes`, which leaves the buffer as it was; an edit that fits allocates
    /// nothing and still succeeds.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_memory_leaves_buffers_alone() {
        isolated(
            concat!(module_path!(), "::out_of_memory_leaves_buffers_alone"),
            || {
                install_refusing_allocator();
                assert!(runtime_pinned());
                let mut buf = Buf::from_bytes(b"kept").unwrap();
                REFUSING.store(true, SeqCst);
                let copied = Buf::from_bytes(b"new").is_none();
                let cloned = buf.as_ref().to_buf().is_none();
                let grown = buf.as_mut().set_bytes(b"longer").is_none();
                let unchanged = buf.as_ref().as_bytes() == b"kept";
                let in_place = buf.as_mut().set_bytes(b"ok").is_some();
                let empty = Buf::from_bytes(b"").is_some();
                REFUSING.store(false, SeqCst);
                assert!(copied && cloned && grown && unchanged && in_place && empty);
                assert_eq!(buf.as_ref().as_bytes(), b"ok");
            },
        );
    }

    /// Without libgit2's runtime nothing can be allocated, but an empty buffer needs
    /// nothing, and disposing it never reaches libgit2.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn without_runtime_only_empty_buffers_exist() {
        isolated(
            concat!(module_path!(), "::without_runtime_only_empty_buffers_exist"),
            || {
                REFUSING.store(true, SeqCst);
                install_refusing_allocator();
                let mut empty = Buf::new();
                assert!(Buf::from_bytes(b"x").is_none());
                assert!(empty.as_mut().set_bytes(b"x").is_none());
                assert!(empty.as_ref().to_buf().is_some());
                assert!(Buf::from_bytes(b"").is_some());
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                drop(empty);
            },
        );
    }
}
