//! Safe wrappers over `include/git2/message.h`.
//!
//! # Cleaning up messages
//!
//! [`prettify`] cleans up a commit message as `git commit` does (git's `stripspace`),
//! into a fresh [`BufVal`] of libgit2's allocator.
//!
//! # Trailers
//!
//! A [`MessageTrailer`] is a `git_message_trailer`: one `key: value` pair of the
//! trailer block that ends a commit message (`Signed-off-by: A U Thor <a@example.com>`).
//! It owns nothing. Its only producer, `git_message_trailers`, copies the message's
//! trailer block into one buffer, cuts it into NUL-terminated keys and values in place,
//! and points each trailer's `key` and `value` into it; the
//! `git_message_trailer_array` it fills owns that block and the buffer of trailers, and
//! `git_message_trailer_array_free` frees both, never a string on its own. Neither
//! pointer is ever NULL in a trailer libgit2 emits, nothing writes through them, and no
//! routine takes a trailer back.
//!
//! So a trailer is a pair of borrowed strings, and Rust spells the borrow: a
//! [`MessageTrailer<'t>`] holds its key and value for `'t`, as a `(&'t CStr, &'t CStr)`
//! would, and keeps the C layout, so a run of them is the `trailers` buffer of a
//! `git_message_trailer_array`. Every handle upholds one invariant, which the safe API
//! keeps and [`MessageTrailer::from_raw`] or a crate-internal `from_ptr` over a
//! C-owned trailer must vouch for: `key` and `value` each address a NUL-terminated
//! string that stays live and unchanged for `'t`. Both are read as [`CStr`]s: a key is
//! ASCII letters, digits and `-`, but a value is whatever bytes the message held.
//!
//! The exclusive [`MessageTrailerMut`] is invariant in `'t`, so it only stores strings
//! that live as long as the trailer says. The wrapper of `git_message_trailer_array`
//! must therefore lend the trailers it owns, whose strings live in its own block, at a
//! `'t` no longer than its shared borrow, and give out no [`MessageTrailerMut`] of them:
//! a string stored through one would have to outlive the array, which a borrow of the
//! array cannot promise.
//!
//! # Trailer arrays
//!
//! A [`MessageTrailerArray`] is a `git_message_trailer_array`, which
//! [`MessageTrailerArray::parse`] fills from a message through `git_message_trailers`:
//! `_trailer_block`, the message's trailer block copied into one buffer of libgit2's
//! allocator, and cut into keys and values in place; `trailers`, a buffer of `count`
//! trailers pointing into that block. libgit2 never allocates the struct itself: the
//! caller holds it by value, `git_message_trailers` writes all three fields on success
//! (without freeing what they held) and leaves them untouched on failure, and
//! `git_message_trailer_array_free` frees both buffers, neither zeroing the struct nor
//! freeing it. So Rust holds it by value too, as a [`MessageTrailerArrayVal`], built
//! empty by [`MessageTrailerArray::new`] for libgit2 to fill, and disposed on drop.
//!
//! Every handle upholds one invariant, which the safe API keeps and
//! [`MessageTrailerArray::from_raw`] or a crate-internal `from_ptr` over a C-owned
//! struct must vouch for: `_trailer_block` is NULL or a live buffer of libgit2's
//! allocator that the array owns; `trailers` is NULL or a live buffer of libgit2's
//! allocator, owned by the array, holding at least `count` trailers whose `key` and
//! `value` address NUL-terminated strings that live, unchanged, as long as the array
//! owns that buffer (libgit2's live in the block). A NULL `trailers` reads as no
//! trailer, whatever `count` says. libgit2 leaves `trailers` NULL and `count` 0 when a
//! message has no trailer, and the block allocated even then.
//!
//! The block and the trailers are one resource: the trailers borrow the block, and
//! `count` sizes the trailers. So [`MessageTrailerArrayRef::trailers`] lends them at
//! the array's shared borrow, as the shared handles they are (see above), and
//! [`MessageTrailerArrayMut`] moves the three fields only together: it takes them out
//! into an owner of their own, leaving the empty array
//! ([`take`](MessageTrailerArrayMut::take)), or disposes them and moves another
//! array's in ([`set`](MessageTrailerArrayMut::set),
//! [`clear`](MessageTrailerArrayMut::clear)). Nothing writes a field alone, and
//! nothing writes the buffers once filled.
//!
//! An empty array (both buffers NULL) never reaches libgit2: nothing allocated it, so
//! nothing has initialised the library on its behalf, and even `git__free(NULL)` reads
//! the allocator a concurrent first `git_libgit2_init` writes. Every other array's
//! buffers came from libgit2's allocator, whose allocation initialised the library
//! first.
//!
//! An array is `Send` and `Sync`, as a `Vec` of strings is: it owns its two buffers,
//! which any thread may free into libgit2's allocator, keeps no thread state and no
//! pointer to anything else, and its shared handle only reads.
//!
//! libgit2 defect, out of the wrapper's reach: when growing the trailer buffer fails,
//! `git_message_trailers` writes through the NULL `git_array_alloc` returns, so it
//! crashes on allocation failure instead of failing.

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull, addr_of};

use ffibox::{CBorrowedPtr, CCell, CDispose, CSlice, CVal, define_ctype};

use crate::ffi;
use crate::libgit2::core::buffer::{Buf, BufVal};
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

/// `message` cleaned up as git cleans up a commit message: trailing whitespace is
/// removed from every line, runs of blank lines are collapsed into one, blank lines at
/// the start and end are removed, and the result ends with a newline unless it is empty.
/// With a `comment` byte, the lines that start with it are removed first (git's
/// `--strip-comments`, `#` by default there); a comment line is any line whose first
/// byte is the comment byte, whitespace before it making it a line like any other.
///
/// Pins the runtime first.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::object::message::prettify;
///
/// let message = prettify(c"\n\nsubject  \n\n\n# comment\nbody\t\n\n", Some(b'#')).unwrap();
/// assert_eq!(message.as_ref().as_bytes(), b"subject\n\nbody\n");
/// let kept = prettify(c"subject\n# comment", None).unwrap();
/// assert_eq!(kept.as_ref().as_bytes(), b"subject\n# comment\n");
/// ```
///
/// Wraps: `git_message_prettify`
///
/// # Errors
///
/// Fails, with libgit2's error, when the result cannot be allocated or libgit2 cannot be
/// initialised.
pub fn prettify(message: &CStr, comment: Option<u8>) -> Result<BufVal, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut out = Buf::new();
    let buf = out.as_mut().as_mut_ptr();
    let (strip, comment) = match comment {
        Some(c) => (1, c.cast_signed()),
        None => (0, 0),
    };
    // SAFETY: `buf` addresses the fresh, empty buffer `out` holds, which libgit2 fills
    // (`git_buf_fromstr`) or leaves empty; `message` is NUL-terminated and read for the
    // call only. The runtime is pinned, so allocations and errors reach live state.
    let rc = unsafe { ffi::git_message_prettify(buf, message.as_ptr(), strip, comment) };
    if rc < 0 { Err(last()) } else { Ok(out) }
}

/// One trailer of a commit message (`git_message_trailer`): a key and its value,
/// borrowed for `'t` from whatever holds the strings.
///
/// Held by value; built from two strings by [`new`](Self::new), or adopted from C with
/// [`from_raw`](Self::from_raw). The getters live on the shared
/// [`as_ref`](Self::as_ref) handle and the setters on the exclusive
/// [`as_mut`](Self::as_mut) one.
///
/// ```
/// use libgit2::libgit2::object::message::MessageTrailer;
///
/// let mut trailer = MessageTrailer::new(c"Signed-off-by", c"A U Thor <a@example.com>");
/// assert_eq!(trailer.as_ref().key(), c"Signed-off-by");
/// trailer.as_mut().set_value(c"C O Mitter <c@example.com>");
/// assert_eq!(trailer.as_ref().value(), c"C O Mitter <c@example.com>");
/// ```
///
/// The strings must outlive the trailer:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::object::message::MessageTrailer;
/// use std::ffi::CString;
///
/// let trailer = {
///     let value = CString::new("A U Thor").unwrap();
///     MessageTrailer::new(c"Signed-off-by", &value)
/// };
/// let _ = trailer.as_ref().value();
/// ```
///
/// and a string read from it lives as long as they do, not longer:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::object::message::MessageTrailer;
/// use std::ffi::CString;
///
/// let key = CString::new("Acked-by").unwrap();
/// let trailer = MessageTrailer::new(&key, c"A U Thor");
/// let read = trailer.as_ref().key();
/// drop(key);
/// let _ = read;
/// ```
///
/// The exclusive handle is invariant in `'t`, so one cannot be shrunk to store a
/// shorter-lived string than its trailer was built for:
///
/// ```compile_fail
/// use libgit2::libgit2::object::message::MessageTrailerMut;
///
/// fn shrink<'a, 't>(m: MessageTrailerMut<'a, 'static>) -> MessageTrailerMut<'a, 't> {
///     m
/// }
/// ```
///
/// The shared handle only reads:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::object::message::MessageTrailer;
///
/// let trailer = MessageTrailer::new(c"Signed-off-by", c"A U Thor");
/// trailer.as_ref().set_key(c"Acked-by");
/// ```
///
/// `Copy`, `Send` and `Sync`, as a pair of `&'t CStr`s is: nothing writes through the
/// strings, and only the exclusive handle writes the fields.
///
/// Wraps: `git_message_trailer`
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct MessageTrailer<'t>(ffi::git_message_trailer, PhantomData<&'t CStr>);

/// Shared borrow of a [`MessageTrailer`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct MessageTrailerRef<'a, 't>(CBorrowedPtr<'a, MessageTrailer<'t>>);

/// Exclusive borrow of a [`MessageTrailer`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct MessageTrailerMut<'a, 't>(
    MessageTrailerRef<'a, 't>,
    // Invariant in the trailer, and so in `'t`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut MessageTrailer<'t>>,
);

// SAFETY: `MessageTrailer` is `#[repr(transparent)]` over `git_message_trailer` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, MessageTrailer<'t>>` (`MessageTrailerMut` through
// `MessageTrailerRef`) with zero-sized markers and no `Drop`, hand out no reference to
// the trailer, and `MessageTrailerRef` has no write operation. `MessageTrailerMut` is
// invariant in the trailer through its `PhantomData<&'a mut MessageTrailer<'t>>`.
unsafe impl<'t> CCell for MessageTrailer<'t> {
    type C = ffi::git_message_trailer;
    type Mut<'a>
        = MessageTrailerMut<'a, 't>
    where
        Self: 'a;
    type Ref<'a>
        = MessageTrailerRef<'a, 't>
    where
        Self: 'a;
}

// SAFETY: a trailer is two string pointers, each a `&'t CStr`'s (per the handles'
// invariant): shared, immutable borrows of `CStr`, which is `Sync`, so they may be read
// from any thread. Nothing in the struct has thread affinity or interior mutability,
// and nothing is released on drop.
unsafe impl Send for MessageTrailer<'_> {}
// SAFETY: as above; `MessageTrailerMut` is an exclusive borrow of such a value, as
// `&mut MessageTrailer`.
unsafe impl Send for MessageTrailerMut<'_, '_> {}

// SAFETY: as above, and shared access only reads the fields (every write goes through
// `MessageTrailerMut`, which excludes other handles), so concurrent readers race
// nothing.
unsafe impl Sync for MessageTrailer<'_> {}
// SAFETY: as above; `MessageTrailerRef` is a shared borrow, as `&MessageTrailer`.
unsafe impl Send for MessageTrailerRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for MessageTrailerRef<'_, '_> {}
// SAFETY: `&MessageTrailerMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for MessageTrailerMut<'_, '_> {}

impl<'t> MessageTrailer<'t> {
    /// The trailer `key: value`, borrowing both strings for `'t`.
    ///
    /// libgit2 does not check a trailer it is not parsing: any key and value are kept
    /// as given, empty ones included.
    #[must_use]
    pub fn new(key: &'t CStr, value: &'t CStr) -> Self {
        Self(
            ffi::git_message_trailer {
                key: key.as_ptr(),
                value: value.as_ptr(),
            },
            PhantomData,
        )
    }

    /// Adopts a `git_message_trailer` C code filled, as `git_message_trailers` fills
    /// the trailers of a `git_message_trailer_array`, borrowing its strings for `'t`.
    ///
    /// # Safety
    ///
    /// `raw.key` and `raw.value` must each be non-NULL and address a NUL-terminated
    /// string that stays live, and that nothing writes, for `'t`. For a trailer of a
    /// `git_message_trailer_array`, `'t` must end before the array is freed with
    /// `git_message_trailer_array_free`, which frees the strings.
    #[must_use]
    pub unsafe fn from_raw(raw: ffi::git_message_trailer) -> Self {
        Self(raw, PhantomData)
    }

    /// The raw `git_message_trailer`, its two pointers borrowed for `'t`; nothing is to
    /// be freed.
    #[must_use]
    pub fn into_raw(self) -> ffi::git_message_trailer {
        self.0
    }

    /// Shared handle to the trailer — the getters, and the pointer for C calls that
    /// read it.
    #[must_use]
    pub fn as_ref(&self) -> MessageTrailerRef<'_, 't> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        MessageTrailerRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the trailer — the setters.
    pub fn as_mut(&mut self) -> MessageTrailerMut<'_, 't> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        MessageTrailerMut(MessageTrailerRef(ptr), PhantomData)
    }
}

impl fmt::Debug for MessageTrailer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'t> MessageTrailerRef<'_, 't> {
    /// Borrows the trailer at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live trailer that stays live, and that nothing
    /// writes, for `'a`, whose `key` and `value` are non-NULL and address
    /// NUL-terminated strings that stay live and unchanged for `'t`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *const ffi::git_message_trailer) -> Option<Self> {
        // SAFETY: non-NULL per `NonNull::new`; live for `'a` per this function's
        // contract; `MessageTrailer` is transparent over `git_message_trailer`.
        NonNull::new(ptr.cast_mut().cast::<MessageTrailer<'t>>())
            .map(|p| Self(unsafe { CBorrowedPtr::new(p) }))
    }

    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_message_trailer *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_message_trailer {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The trailer's key, without the separator or the whitespace around it
    /// (`Signed-off-by`). Borrowed for `'t`, as the trailer holds it.
    ///
    /// Field: `git_message_trailer.key`
    #[must_use]
    pub fn key(&self) -> &'t CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live trailer. Per the
        // handles' invariant, which `new`, `set_key`, `from_raw` and `from_ptr` keep,
        // `key` is non-NULL and addresses a NUL-terminated string live and unchanged
        // for `'t`.
        let key_ptr2 = unsafe { addr_of!((*p).key) };
        // SAFETY: as above.
        let read2 = unsafe { key_ptr2.read() };
        // SAFETY: as above.
        unsafe { CStr::from_ptr(read2) }
    }

    /// The trailer's value, from after the separator and the whitespace following it,
    /// continuation lines included with their newlines (`A U Thor <a@example.com>`).
    /// Borrowed for `'t`, as the trailer holds it.
    ///
    /// Field: `git_message_trailer.value`
    #[must_use]
    pub fn value(&self) -> &'t CStr {
        let p = self.as_ptr();
        // SAFETY: as `key`, for `value`, which `new`, `set_value`, `from_raw` and
        // `from_ptr` keep non-NULL and live for `'t`.
        let value_ptr2 = unsafe { addr_of!((*p).value) };
        // SAFETY: as above.
        let read2 = unsafe { value_ptr2.read() };
        // SAFETY: as above.
        unsafe { CStr::from_ptr(read2) }
    }
}

impl fmt::Debug for MessageTrailerRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageTrailer")
            .field("key", &self.key())
            .field("value", &self.value())
            .finish()
    }
}

impl<'t> MessageTrailerMut<'_, 't> {
    /// Writable pointer to the C struct.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_message_trailer {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> MessageTrailerRef<'_, 't> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> MessageTrailerMut<'_, 't> {
        MessageTrailerMut(self.0, PhantomData)
    }

    /// Replaces the key with `key`, borrowed for `'t`. The previous key is only
    /// borrowed, so nothing is released.
    ///
    /// Field: `git_message_trailer.key`
    pub fn set_key(&mut self, key: &'t CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let key_ptr = unsafe { &raw mut (*p).key };
        // SAFETY: a write through the exclusive handle's pointer to a live trailer; the
        // pointer stored is a `&'t CStr`'s, non-NULL and live and unchanged for `'t`,
        // which keeps the handles' invariant.
        unsafe { key_ptr.write(key.as_ptr()) }
    }

    /// Replaces the value with `value`, borrowed for `'t`. The previous value is only
    /// borrowed, so nothing is released.
    ///
    /// Field: `git_message_trailer.value`
    pub fn set_value(&mut self, value: &'t CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let value_ptr = unsafe { &raw mut (*p).value };
        // SAFETY: as `set_key`, for `value`.
        unsafe { value_ptr.write(value.as_ptr()) }
    }
}

impl fmt::Debug for MessageTrailerMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A trailer is the C struct itself: two pointers, and a handle one pointer.
const _: () = assert!(
    size_of::<MessageTrailer<'static>>() == size_of::<ffi::git_message_trailer>()
        && align_of::<MessageTrailer<'static>>() == align_of::<ffi::git_message_trailer>()
        && size_of::<MessageTrailerRef<'static, 'static>>() == size_of::<*const ()>()
);

define_ctype!(
    /// The trailers of a commit message (`git_message_trailer_array`): the message's
    /// trailer block, copied and cut into keys and values, and the `count` trailers
    /// pointing into it, both in libgit2's allocator and owned by the struct.
    ///
    /// Held by value inside a [`MessageTrailerArrayVal`], which [`parse`](Self::parse)
    /// fills from a message, [`new`](Self::new) builds empty and
    /// [`from_raw`](Self::from_raw) adopts from C; read it through the owner's
    /// [`as_ref`](CVal::as_ref) handle:
    ///
    /// ```
    /// use libgit2::libgit2::object::message::MessageTrailerArray;
    ///
    /// # if cfg!(miri) { return; }
    /// let message = c"Subject\n\nBody.\n\nSigned-off-by: A U Thor <a@example.com>\n";
    /// let array = MessageTrailerArray::parse(message).unwrap();
    /// let trailers = array.as_ref().trailers();
    /// assert_eq!(trailers.len(), 1);
    /// let first = trailers.get(0).unwrap();
    /// assert_eq!(
    ///     (first.key(), first.value()),
    ///     (c"Signed-off-by", c"A U Thor <a@example.com>")
    /// );
    /// ```
    ///
    /// The trailers' strings live in the array's block, so neither a trailer nor a
    /// string read from one outlives the array:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::object::message::MessageTrailerArray;
    ///
    /// let array = MessageTrailerArray::parse(c"S\n\nKey: value\n").unwrap();
    /// let key = array.as_ref().trailers().get(0).unwrap().key();
    /// drop(array);
    /// let _ = key;
    /// ```
    ///
    /// nor survives the array being emptied or replaced through its exclusive handle:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::object::message::MessageTrailerArray;
    ///
    /// let mut array = MessageTrailerArray::parse(c"S\n\nKey: value\n").unwrap();
    /// let trailers = array.as_ref().trailers();
    /// array.as_mut().clear();
    /// let _ = trailers.get(0);
    /// ```
    ///
    /// No exclusive handle reaches a trailer of the array: one could store a string
    /// that does not live as long as the block, and the run is a shared one.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::object::message::MessageTrailerArray;
    ///
    /// let mut array = MessageTrailerArray::parse(c"S\n\nKey: value\n").unwrap();
    /// let _ = array.as_mut().trailers_mut();
    /// ```
    ///
    /// Not `Clone`: it owns its buffers, and libgit2 has no copy routine.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::object::message::{MessageTrailerArray, MessageTrailerArrayVal};
    ///
    /// let array = MessageTrailerArray::new();
    /// let copy: MessageTrailerArrayVal = array.clone();
    /// ```
    ///
    /// The owner and its handles are `Send` and `Sync` (see the
    /// [module docs](self#trailer-arrays)).
    ///
    /// Wraps: git_message_trailer_array
    MessageTrailerArray,
    MessageTrailerArrayRef,
    MessageTrailerArrayMut,
    ffi::git_message_trailer_array
);

// SAFETY: an array owns its block and its trailer buffer, both in libgit2's allocator,
// which any thread may free into; the trailers point into the block (or at strings that
// outlive the array, per `from_raw`'s contract), and nothing in it has thread affinity
// or points at a repository or any other thread's state.
unsafe impl Send for MessageTrailerArray {}
// SAFETY: shared access only reads the three fields and the strings, which nothing
// writes once filled: every write goes through `MessageTrailerArrayMut`, which excludes
// every other handle.
unsafe impl Sync for MessageTrailerArray {}
// SAFETY: `MessageTrailerArrayRef` is a shared borrow of a `Sync` array, as
// `&MessageTrailerArray`.
unsafe impl Send for MessageTrailerArrayRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for MessageTrailerArrayRef<'_> {}
// SAFETY: `MessageTrailerArrayMut` is an exclusive borrow of a `Send` array, as
// `&mut MessageTrailerArray`.
unsafe impl Send for MessageTrailerArrayMut<'_> {}
// SAFETY: `&MessageTrailerArrayMut` only reaches the shared handle, through `as_ref`.
unsafe impl Sync for MessageTrailerArrayMut<'_> {}

/// The empty array: no block, no trailers, as `git_message_trailer_array arr = {0}`.
const EMPTY: ffi::git_message_trailer_array = ffi::git_message_trailer_array {
    trailers: ptr::null_mut(),
    count: 0,
    _trailer_block: ptr::null_mut(),
};

/// Releases an array's buffers through `git_message_trailer_array_free`, which frees
/// the block and the trailers with `git__free`, leaving the struct itself, Rust's
/// storage, alone (and not zeroed).
///
/// An empty array (both buffers NULL) is not passed to C: it owns nothing, and the call
/// would be two `git__free(NULL)`, which read `git__allocator` without synchronisation
/// against the first `git_libgit2_init` another thread may be running.
///
/// Wraps: `git_message_trailer_array_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct MessageTrailerArrayFree;

// SAFETY: `git_message_trailer_array_free` frees `_trailer_block` and `trailers` once
// each, and never the struct. It accepts every state the safe API reaches: arrays
// `git_message_trailers` filled (whose allocation initialised the library, so the
// allocator it reads is settled) and ones adopted through `from_raw` under its
// contract; `new`, `take` and `clear` leave the empty array, which owns nothing and is
// released without C; `set` only moves in another owner's array.
unsafe impl CDispose<MessageTrailerArray> for MessageTrailerArrayFree {
    #[inline]
    unsafe fn c_dispose(&self, ptr: NonNull<MessageTrailerArray>) {
        let raw: *mut ffi::git_message_trailer_array = ptr.as_ptr().cast();
        // SAFETY: per `c_dispose`'s contract `raw` addresses a live
        // `git_message_trailer_array`.
        let trailer_block_ptr = unsafe { addr_of!((*raw)._trailer_block) };
        // SAFETY: as above.
        let read2 = unsafe { trailer_block_ptr.read() };
        // SAFETY: as above.
        let trailers_ptr = unsafe { addr_of!((*raw).trailers) };
        // SAFETY: as above.
        let (block, trailers) = unsafe { (read2, trailers_ptr.read()) };
        if block.is_null() && trailers.is_null() {
            return;
        }
        // SAFETY: `raw` is live and, per the handles' invariant, owns both buffers (or
        // NULL), from libgit2's allocator; the caller gives the value up, so each is
        // freed once.
        unsafe { ffi::git_message_trailer_array_free(raw) }
    }
}

/// An owned [`MessageTrailerArray`], held inline; its block and trailers are released
/// by `git_message_trailer_array_free` on drop.
pub type MessageTrailerArrayVal = CVal<MessageTrailerArray, MessageTrailerArrayFree>;

impl MessageTrailerArray {
    /// An empty array: no trailer, nothing allocated, as
    /// `git_message_trailer_array arr = {0}`.
    pub fn new() -> MessageTrailerArrayVal {
        // SAFETY: an empty value owns nothing, which `git_message_trailer_array_free` accepts.
        unsafe { CVal::new(Self(EMPTY, PhantomData)) }
    }

    /// The trailers of `message`, as git finds them: the last paragraph after the
    /// title, before any `---` patch line and trailing comments, when its lines are all
    /// `key: value` trailers (or a quarter are, one of them git-generated, such as
    /// `Signed-off-by:`). Each key is ASCII letters, digits and `-`, read without the
    /// whitespace before the `:`; each value starts after the whitespace following it
    /// and takes in the lines that follow indented by a space, newlines included.
    /// Lines of the block that are not a trailer are skipped, and a message without
    /// such a block has no trailer. The block is copied: the array does not borrow
    /// `message`.
    ///
    /// Pins the runtime first.
    ///
    /// Wraps: `git_message_trailers`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when the block cannot be allocated or libgit2 cannot be
    /// initialised. libgit2 does not check the growth of its trailer buffer (`git_array_alloc`):
    /// should that allocation fail, it writes the trailer through NULL and the process crashes,
    /// which only exhausting memory between the block's allocation and the buffer's can bring
    /// about.
    pub fn parse(message: &CStr) -> Result<MessageTrailerArrayVal, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut array = Self::new();
        let place = array.as_mut().as_mut_ptr();
        // SAFETY: `place` addresses the live, empty array the exclusive borrow just
        // lent, which libgit2 overwrites whole only on success (nothing to leak), and
        // leaves empty on failure; `message` is NUL-terminated and only read during the
        // call. The runtime is pinned, so the allocations go through an initialised
        // allocator.
        let rc = unsafe { ffi::git_message_trailers(place, message.as_ptr()) };
        if rc < 0 {
            return Err(last());
        }
        Ok(array)
    }

    /// Adopts a `git_message_trailer_array` C code filled, as `git_message_trailers`
    /// fills one: its block and trailers become the result's, released by
    /// `git_message_trailer_array_free` on drop.
    ///
    /// An array whose `trailers` is NULL holds no trailer, whatever `count` says.
    ///
    /// # Safety
    ///
    /// `raw._trailer_block` and `raw.trailers` must each be NULL or a live allocation
    /// from libgit2's current allocator, which the caller gives up: nothing else may
    /// free them, nor write them while the result lives. When `raw.trailers` is not
    /// NULL, it must hold at least `raw.count` initialised trailers, each of whose
    /// `key` and `value` is non-NULL and addresses a NUL-terminated string that stays
    /// live, and that nothing writes, until the result releases the trailers (strings
    /// in `raw._trailer_block` qualify, as libgit2's do).
    pub unsafe fn from_raw(raw: ffi::git_message_trailer_array) -> MessageTrailerArrayVal {
        // SAFETY: the caller gives up `raw`, which owns what it points at (this
        // function's contract).
        unsafe { CVal::new(Self(raw, PhantomData)) }
    }

    /// The raw `git_message_trailer_array`, ownership included: C code releases it
    /// with `git_message_trailer_array_free`.
    #[must_use]
    pub fn into_raw(self) -> ffi::git_message_trailer_array {
        self.0
    }
}

impl<'a> MessageTrailerArrayRef<'a> {
    /// The number of trailers; 0 without a trailer buffer, whatever `count` says, as
    /// [`trailers`](Self::trailers) reads it.
    ///
    /// Field: `git_message_trailer_array.count`
    #[must_use]
    pub fn len(&self) -> usize {
        let array = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to a live
        // `git_message_trailer_array`; any `size_t` is a valid `usize`.
        let trailers_ptr2 = unsafe { addr_of!((*array).trailers) };
        // SAFETY: as above.
        let read2 = unsafe { trailers_ptr2.read() };
        // SAFETY: as above.
        let count_ptr = unsafe { addr_of!((*array).count) };
        // SAFETY: as above.
        let (trailers, count) = unsafe { (read2, count_ptr.read()) };
        if trailers.is_null() { 0 } else { count }
    }

    /// Whether the array holds no trailer.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The trailers, in message order, borrowed in place for as long as the array is:
    /// shared handles, whose keys and values live in the array's block, and so for as
    /// long too. No exclusive handle reaches them (see the
    /// [module docs](self#trailer-arrays)).
    ///
    /// Field: `git_message_trailer_array.trailers`
    #[must_use]
    pub fn trailers(&self) -> CSlice<'a, MessageTrailer<'a>> {
        let len = self.len();
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let trailers_ptr = unsafe { &raw const (*self.as_ptr()).trailers };
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_message_trailer_array`.
        let trailers = unsafe { trailers_ptr.read() };
        match NonNull::new(trailers.cast::<MessageTrailer<'a>>()) {
            // SAFETY: per the handles' invariant a non-NULL `trailers` addresses at
            // least `count` initialised trailers, `MessageTrailer` being transparent
            // over `git_message_trailer`, owned by the array, so they live as long as
            // it does, past `'a`, as do the strings they point at; nothing writes
            // them while the array is shared. Their strings are non-NULL, so each is a
            // valid `MessageTrailer<'a>`.
            Some(trailers) => unsafe { CSlice::from_raw_parts(trailers, len) },
            // NULL is the empty array; a run may not start at NULL.
            // SAFETY: zero elements at an aligned dangling pointer.
            None => unsafe { CSlice::from_raw_parts(NonNull::dangling(), 0) },
        }
    }
}

impl fmt::Debug for MessageTrailerArrayRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.trailers().iter()).finish()
    }
}

impl fmt::Debug for MessageTrailerArrayMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl MessageTrailerArrayMut<'_> {
    /// Takes the block and the trailers out into an owner of their own, leaving the
    /// empty array, which libgit2 can fill again without leaking what it overwrites.
    ///
    /// Field: `git_message_trailer_array`._`trailer_block`
    pub fn take(&mut self) -> MessageTrailerArrayVal {
        let array = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses a live, initialised
        // `git_message_trailer_array`, plain data read out whole and replaced by the
        // empty array in one place, so its buffers have exactly one owner throughout.
        let raw = unsafe { array.replace(EMPTY) };
        // SAFETY: `raw` held the handles' invariant in this array, whose buffers it
        // takes over; nothing else frees them now that the place is empty.
        unsafe { MessageTrailerArray::from_raw(raw) }
    }

    /// Replaces the block and the trailers with `array`'s, releasing the ones held
    /// before through `git_message_trailer_array_free`.
    ///
    /// Field: `git_message_trailer_array`._`trailer_block`
    pub fn set(&mut self, array: MessageTrailerArrayVal) {
        let raw = array.into_inner().into_raw();
        let place = self.as_mut_ptr();
        // SAFETY: as in `take`: the live array is read out and replaced whole by `raw`,
        // which `array`'s owner gave up and which holds the handles' invariant.
        let old = unsafe { place.replace(raw) };
        // SAFETY: `old` held the invariant here and nothing else owns it any more.
        drop(unsafe { MessageTrailerArray::from_raw(old) });
    }

    /// Releases the block and the trailers, leaving the empty array.
    pub fn clear(&mut self) {
        drop(self.take());
    }
}

// A `MessageTrailerArray` is a `git_message_trailer_array`, so C reads and writes a
// value Rust holds in place, and the owner adds nothing to it.
const _: () = assert!(
    size_of::<MessageTrailerArray>() == size_of::<ffi::git_message_trailer_array>()
        && align_of::<MessageTrailerArray>() == align_of::<ffi::git_message_trailer_array>()
        && size_of::<MessageTrailerArrayVal>() == size_of::<ffi::git_message_trailer_array>()
);

#[cfg(test)]
mod unit_tests {
    use core::mem::{MaybeUninit, size_of};
    use core::ptr;
    use std::ffi::CString;
    use std::thread;

    use super::*;
    use crate::libgit2::util::sys::errors::runtime_pinned;

    const MESSAGE: &CStr = c"Subject\n\nBody.\n\nSigned-off-by: A U Thor <a@example.com>\nCo-authored-by : C O Mitter\n  continued\n";

    /// `git_message_trailers` over `message`, raw, for the adoption tests; the caller
    /// frees the array.
    fn parse_raw(message: &CStr) -> ffi::git_message_trailer_array {
        assert!(runtime_pinned());
        let mut arr = MaybeUninit::<ffi::git_message_trailer_array>::zeroed();
        // SAFETY: `arr` is writable storage, which libgit2 fills on success; `message`
        // is NUL-terminated and only read.
        let rc = unsafe { ffi::git_message_trailers(arr.as_mut_ptr(), message.as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: a 0 return means every field was written.
        unsafe { arr.assume_init() }
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(
            size_of::<MessageTrailer<'_>>(),
            size_of::<ffi::git_message_trailer>()
        );
        assert_eq!(
            align_of::<MessageTrailer<'_>>(),
            align_of::<ffi::git_message_trailer>()
        );
        assert_eq!(
            size_of::<MessageTrailerRef<'_, '_>>(),
            size_of::<*const ()>()
        );
        assert_eq!(
            size_of::<MessageTrailerMut<'_, '_>>(),
            size_of::<*const ()>()
        );
        assert_eq!(
            size_of::<Option<MessageTrailerRef<'_, '_>>>(),
            size_of::<*const ()>()
        );
    }

    /// `new` stores the strings' own pointers, `into_raw` gives them back, and the
    /// setters replace one field each; a copy is independent.
    #[test]
    fn fields_round_trip_through_the_handles() {
        let (key, value) = (c"Acked-by", c"A U Thor");
        let mut trailer = MessageTrailer::new(key, value);
        let raw = trailer.into_raw();
        assert_eq!((raw.key, raw.value), (key.as_ptr(), value.as_ptr()));
        assert_eq!(trailer.as_ref().as_ptr(), ptr::from_ref(&trailer).cast());

        let copy = trailer;
        let new_key = CString::new("Reviewed-by").unwrap();
        {
            let mut m = trailer.as_mut();
            m.as_mut().set_key(c"Tested-by");
            assert_eq!(m.as_ref().key(), c"Tested-by");
            m.set_value(c"");
            assert_eq!(m.as_mut().as_mut_ptr().cast_const(), m.as_ref().as_ptr());
        }
        assert_eq!(trailer.as_ref().key(), c"Tested-by");
        assert_eq!(trailer.as_ref().value(), c"");
        assert_eq!(copy.as_ref().key(), key);
        assert_eq!(copy.as_ref().value(), value);

        let mut short = MessageTrailer::new(&new_key, value);
        short.as_mut().set_value(&new_key);
        assert_eq!(short.as_ref().value(), c"Reviewed-by");
    }

    /// `Debug` prints the key and value through every form.
    #[test]
    fn debug_prints_key_and_value() {
        let mut trailer = MessageTrailer::new(c"Fixes", c"#123");
        let expected = r##"MessageTrailer { key: "Fixes", value: "#123" }"##;
        assert_eq!(format!("{trailer:?}"), expected);
        assert_eq!(format!("{:?}", trailer.as_ref()), expected);
        assert_eq!(format!("{:?}", trailer.as_mut()), expected);
    }

    /// Trailers libgit2 parses read back through borrowed handles (`from_ptr`) and
    /// adopted copies (`from_raw`), both of which end before the array is freed (`ASan`
    /// flags a read past it).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn parsed_trailers_read_back() {
        let array = MessageTrailerArray::parse(MESSAGE).unwrap();
        let trailers = array.as_ref().trailers();
        assert_eq!(trailers.len(), 2);
        let expected = [
            (c"Signed-off-by", c"A U Thor <a@example.com>"),
            (c"Co-authored-by", c"C O Mitter\n  continued"),
        ];
        for (i, (key, value)) in expected.into_iter().enumerate() {
            let (handle, copy) = {
                // SAFETY: `i < len`, so the trailer is in the array's buffer; libgit2 set
                // both strings, which live in the array's block until `array` drops, after
                // the handle and the copy are last used.
                let p = unsafe { trailers.as_ptr().add(i) };
                // SAFETY: as above.
                let value2 = unsafe { MessageTrailerRef::from_ptr(p) };
                // SAFETY: as above.
                let read2 = unsafe { p.read() };
                // SAFETY: as above.
                unsafe { (value2.unwrap(), MessageTrailer::from_raw(read2)) }
            };
            assert_eq!((handle.key(), handle.value()), (key, value));
            assert_eq!((copy.as_ref().key(), copy.as_ref().value()), (key, value));
        }
        // SAFETY: NULL is refused before any handle exists.
        assert!(unsafe { MessageTrailerRef::from_ptr(ptr::null()) }.is_none());
    }

    #[test]
    fn array_layout_matches_c() {
        assert_eq!(
            size_of::<MessageTrailerArray>(),
            size_of::<ffi::git_message_trailer_array>()
        );
        assert_eq!(
            align_of::<MessageTrailerArray>(),
            align_of::<ffi::git_message_trailer_array>()
        );
        assert_eq!(
            size_of::<MessageTrailerArrayVal>(),
            size_of::<ffi::git_message_trailer_array>()
        );
        assert_eq!(
            size_of::<MessageTrailerArrayRef<'_>>(),
            size_of::<*const ()>()
        );
        assert_eq!(size_of::<MessageTrailerArrayFree>(), 0);
    }

    /// The empty array owns nothing, reads as no trailer, and is emptied, taken and
    /// dropped without C, so Miri runs it.
    #[test]
    fn empty_arrays_never_reach_c() {
        let mut array = MessageTrailerArray::new();
        let raw = array.as_ref().as_ptr();
        // SAFETY: reads a live array the owner holds.
        let trailers2 = unsafe { (*raw).trailers };
        // SAFETY: as above.
        let count2 = unsafe { (*raw).count };
        // SAFETY: as above.
        let fields = unsafe { (trailers2, count2, (*raw)._trailer_block) };
        assert_eq!(fields, (ptr::null_mut(), 0, ptr::null_mut()));
        assert!(array.as_ref().is_empty());
        assert!(array.as_ref().trailers().is_empty());
        assert_eq!(format!("{:?}", array.as_ref()), "[]");
        assert_eq!(format!("{:?}", array.as_mut()), "[]");
        array.as_mut().clear();
        let taken = array.as_mut().take();
        assert!(taken.as_ref().is_empty());
        array.as_mut().set(taken);
        assert!(array.as_ref().is_empty());
        let raw = array.into_inner().into_raw();
        assert!(raw.trailers.is_null() && raw._trailer_block.is_null());
    }

    /// A NULL trailer buffer reads as empty whatever `count` says, and is disposed
    /// without C (Miri).
    #[test]
    fn null_trailers_read_as_empty() {
        let raw = ffi::git_message_trailer_array {
            trailers: ptr::null_mut(),
            count: 7,
            _trailer_block: ptr::null_mut(),
        };
        // SAFETY: both buffers are NULL, which owns nothing.
        let array = unsafe { MessageTrailerArray::from_raw(raw) };
        assert_eq!(array.as_ref().len(), 0);
        assert!(array.as_ref().trailers().get(0).is_none());
        assert_eq!(array.as_ref().trailers().iter().count(), 0);
    }

    /// An array C filled is adopted and freed once by the owner (`ASan`: no double
    /// free; `LSan`: no leak); one the owner gives up is freed by C.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopted_arrays_are_released_once() {
        // SAFETY: filled by `git_message_trailers`, whose buffers the owner takes over.
        let adopted = unsafe { MessageTrailerArray::from_raw(parse_raw(MESSAGE)) };
        assert_eq!(adopted.as_ref().len(), 2);
        let parsed = MessageTrailerArray::parse(MESSAGE).unwrap();
        let (a, b) = (adopted.as_ref().trailers(), parsed.as_ref().trailers());
        assert!(
            a.iter()
                .zip(b.iter())
                .all(|(x, y)| (x.key(), x.value()) == (y.key(), y.value()))
        );
        drop(adopted);

        let mut raw = parsed.into_inner().into_raw();
        assert_eq!(raw.count, 2);
        assert!(!raw._trailer_block.is_null());
        // SAFETY: the buffers the owner gave up, freed once.
        unsafe { ffi::git_message_trailer_array_free(&raw mut raw) };

        // An adopted array with a block but no trailer buffer frees the block alone.
        // SAFETY: filled by `git_message_trailers`, freed by the owner.
        let blank = unsafe { MessageTrailerArray::from_raw(parse_raw(c"Subject\n\nBody.\n")) };
        assert!(blank.as_ref().is_empty());
        let (trailers, block) = {
            let raw = blank.as_ref().as_ptr();
            // SAFETY: reads a live array.
            let trailers2 = unsafe { (*raw).trailers };
            // SAFETY: as above.
            unsafe { (trailers2, (*raw)._trailer_block) }
        };
        assert!(trailers.is_null() && !block.is_null());
    }

    /// Taking, replacing and clearing move the block and the trailers together, each
    /// freed once, by whichever owner holds them last.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn take_set_and_clear_move_the_buffers_together() {
        let mut array = MessageTrailerArray::parse(MESSAGE).unwrap();
        let block = |a: MessageTrailerArrayRef<'_>| {
            // SAFETY: reads a live array.
            let value = unsafe { addr_of!((*a.as_ptr())._trailer_block) };
            // SAFETY: as above.
            unsafe { value.read() }
        };
        let before = block(array.as_ref());
        let taken = array.as_mut().take();
        assert!(array.as_ref().is_empty());
        assert!(block(array.as_ref()).is_null());
        assert_eq!(block(taken.as_ref()), before);
        assert_eq!(
            taken.as_ref().trailers().get(1).unwrap().key(),
            c"Co-authored-by"
        );

        array.as_mut().set(taken);
        assert_eq!(block(array.as_ref()), before);
        array
            .as_mut()
            .set(MessageTrailerArray::parse(c"S\n\nOnly: one\n").unwrap());
        assert_eq!(array.as_ref().len(), 1);
        assert_eq!(array.as_ref().trailers().get(0).unwrap().value(), c"one");
        let mut m = array.as_mut();
        m.as_mut().clear();
        assert!(m.as_ref().is_empty());
        m.set(MessageTrailerArray::parse(MESSAGE).unwrap());
        assert_eq!(
            format!("{m:?}"),
            format!(
                "{:?}",
                MessageTrailerArray::parse(MESSAGE).unwrap().as_ref()
            )
        );
        assert!(
            format!("{:?}", array.as_ref()).starts_with("[MessageTrailer { key: \"Signed-off-by\"")
        );
    }

    /// Arrays cross threads: shared ones are read from several at once, and an owner
    /// parsed on one thread is dropped on another (`TSan`).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn arrays_cross_threads() {
        let array = MessageTrailerArray::parse(MESSAGE).unwrap();
        let handle = array.as_ref();
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(move || {
                    for t in handle.trailers().iter() {
                        assert!(!t.key().is_empty());
                    }
                });
            }
        });
        let moved = thread::spawn(move || {
            assert_eq!(array.as_ref().len(), 2);
            MessageTrailerArray::parse(MESSAGE).unwrap()
        })
        .join()
        .unwrap();
        thread::spawn(move || drop(moved)).join().unwrap();
    }

    /// Trailers cross threads and are read from several at once (TSan): they are a
    /// pair of shared strings.
    #[test]
    fn trailers_are_shared_across_threads() {
        let key = CString::new("Signed-off-by").unwrap();
        let trailer = MessageTrailer::new(&key, c"A U Thor");
        let handle = trailer.as_ref();
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(move || {
                    for _ in 0..64 {
                        assert_eq!(handle.key(), c"Signed-off-by");
                        assert_eq!(trailer.as_ref().value(), c"A U Thor");
                    }
                });
            }
        });
        let mut moved = trailer;
        thread::scope(|s| {
            let mut m = moved.as_mut();
            s.spawn(move || m.set_value(c"C O Mitter"));
        });
        assert_eq!(moved.as_ref().value(), c"C O Mitter");
    }

    /// Without libgit2's runtime nothing is cleaned up: the error is synthesised without
    /// calling libgit2. libgit2 cannot initialise under an allocator that refuses every
    /// request; alone in a child, where nothing has initialised it yet.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn prettify_without_runtime_fails_cleanly() {
        use crate::libgit2::util::errors::ErrorClass;
        use crate::libgit2::util::sys::alloc::{Alloc, Allocator};

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";
        const NAME: &str =
            "libgit2::object::message::unit_tests::prettify_without_runtime_fails_cleanly";

        /// Refuses every request, which `Alloc` allows.
        struct Never;

        // SAFETY: every allocation fails, which `Alloc` allows; nothing is ever handed
        // out, so `free` never receives a block.
        unsafe impl Alloc for Never {
            fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                None
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

        if std::env::var(CHILD).as_deref() != Ok(NAME) {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([NAME, "--exact", "--test-threads=1"])
                .env(CHILD, NAME)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "{stdout}"
            );
            return;
        }
        let allocator = Allocator::new::<Never>();
        // SAFETY: `Allocator` is transparent over a `git_allocator` with every member
        // set; libgit2 copies it before anything is allocated in this child.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        let error = prettify(c"subject", None).unwrap_err();
        assert!(!runtime_pinned());
        assert_eq!(error.class(), Some(ErrorClass::Invalid));
        assert_eq!(error.message(), c"library has not been initialized");
    }
}
