//! Safe wrappers over `include/git2/message.h`.
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

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{NonNull, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CCell};

use crate::ffi;

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
/// Wraps: git_message_trailer
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
    type Ref<'a>
        = MessageTrailerRef<'a, 't>
    where
        Self: 'a;
    type Mut<'a>
        = MessageTrailerMut<'a, 't>
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
    pub unsafe fn from_raw(raw: ffi::git_message_trailer) -> Self {
        Self(raw, PhantomData)
    }

    /// The raw `git_message_trailer`, its two pointers borrowed for `'t`; nothing is to
    /// be freed.
    pub fn into_raw(self) -> ffi::git_message_trailer {
        self.0
    }

    /// Shared handle to the trailer — the getters, and the pointer for C calls that
    /// read it.
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

impl<'a, 't> MessageTrailerRef<'a, 't> {
    /// Borrows the trailer at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live trailer that stays live, and that nothing
    /// writes, for `'a`, whose `key` and `value` are non-NULL and address
    /// NUL-terminated strings that stay live and unchanged for `'t`.
    #[allow(dead_code)]
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
    /// Field: git_message_trailer.key
    pub fn key(&self) -> &'t CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live trailer. Per the
        // handles' invariant, which `new`, `set_key`, `from_raw` and `from_ptr` keep,
        // `key` is non-NULL and addresses a NUL-terminated string live and unchanged
        // for `'t`.
        unsafe { CStr::from_ptr(addr_of!((*p).key).read()) }
    }

    /// The trailer's value, from after the separator and the whitespace following it,
    /// continuation lines included with their newlines (`A U Thor <a@example.com>`).
    /// Borrowed for `'t`, as the trailer holds it.
    ///
    /// Field: git_message_trailer.value
    pub fn value(&self) -> &'t CStr {
        let p = self.as_ptr();
        // SAFETY: as `key`, for `value`, which `new`, `set_value`, `from_raw` and
        // `from_ptr` keep non-NULL and live for `'t`.
        unsafe { CStr::from_ptr(addr_of!((*p).value).read()) }
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

impl<'a, 't> MessageTrailerMut<'a, 't> {
    /// Writable pointer to the C struct.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_message_trailer {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
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
    /// Field: git_message_trailer.key
    pub fn set_key(&mut self, key: &'t CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live trailer; the
        // pointer stored is a `&'t CStr`'s, non-NULL and live and unchanged for `'t`,
        // which keeps the handles' invariant.
        unsafe { addr_of_mut!((*p).key).write(key.as_ptr()) }
    }

    /// Replaces the value with `value`, borrowed for `'t`. The previous value is only
    /// borrowed, so nothing is released.
    ///
    /// Field: git_message_trailer.value
    pub fn set_value(&mut self, value: &'t CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: as `set_key`, for `value`.
        unsafe { addr_of_mut!((*p).value).write(value.as_ptr()) }
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

#[cfg(test)]
mod unit_tests {
    use super::*;
    use core::mem::{MaybeUninit, size_of};
    use core::ptr;
    use std::ffi::CString;
    use std::thread;

    use crate::libgit2::util::sys::errors::runtime_pinned;

    const MESSAGE: &CStr = c"Subject\n\nBody.\n\nSigned-off-by: A U Thor <a@example.com>\nCo-authored-by : C O Mitter\n  continued\n";

    /// `git_message_trailers` over `message`; the caller frees the array.
    fn parse(message: &CStr) -> ffi::git_message_trailer_array {
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
            size_of::<MessageTrailer>(),
            size_of::<ffi::git_message_trailer>()
        );
        assert_eq!(
            align_of::<MessageTrailer>(),
            align_of::<ffi::git_message_trailer>()
        );
        assert_eq!(size_of::<MessageTrailerRef>(), size_of::<*const ()>());
        assert_eq!(size_of::<MessageTrailerMut>(), size_of::<*const ()>());
        assert_eq!(
            size_of::<Option<MessageTrailerRef>>(),
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
    /// adopted copies (`from_raw`), both of which end before the array is freed (ASan
    /// flags a read past it).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn parsed_trailers_read_back() {
        let mut arr = parse(MESSAGE);
        assert_eq!(arr.count, 2);
        let expected = [
            (c"Signed-off-by", c"A U Thor <a@example.com>"),
            (c"Co-authored-by", c"C O Mitter\n  continued"),
        ];
        for (i, (key, value)) in expected.into_iter().enumerate() {
            // SAFETY: `i < count`, so the trailer is in the array's buffer; libgit2 set
            // both strings, which live in the array's block until it is freed below,
            // after the handle and the copy are last used.
            let (handle, copy) = unsafe {
                let p = arr.trailers.add(i);
                (
                    MessageTrailerRef::from_ptr(p).unwrap(),
                    MessageTrailer::from_raw(p.read()),
                )
            };
            assert_eq!((handle.key(), handle.value()), (key, value));
            assert_eq!((copy.as_ref().key(), copy.as_ref().value()), (key, value));
        }
        // SAFETY: NULL is refused before any handle exists.
        assert!(unsafe { MessageTrailerRef::from_ptr(ptr::null()) }.is_none());
        // SAFETY: `arr` was filled by `git_message_trailers` and is freed once.
        unsafe { ffi::git_message_trailer_array_free(&mut arr) };
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
}
