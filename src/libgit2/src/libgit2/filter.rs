//! Safe wrappers over `include/git2/filter.h`.
//!
//! # Filter lists
//!
//! A [`FilterList`] is a `git_filter_list`: the filters (line-ending conversion,
//! `$Id$` expansion, or ones registered through `git2/sys/filter.h`) that apply to one
//! path in one direction, with the state each filter keeps for it. It is opaque:
//! libgit2 publishes no field.
//!
//! ## Ownership
//!
//! libgit2 allocates a list in one block, the path inline, and hands the caller sole
//! ownership; nothing counts or copies it. `git_filter_list_free` releases it: it hands
//! each filter's state (the payload its `check` made) to the filter's `cleanup`, frees
//! the entry array, then the block. So Rust owns one through a [`FilterListBox`],
//! released by [`FilterListFree`], and not `Clone`. The constructors are
//! [`FilterList::new`] (an empty list, `git2/sys/filter.h`) and [`FilterList::load`] /
//! [`FilterList::load_ext`] (the filters a path's attributes select), which give `None`
//! when no filter applies: libgit2 then succeeds with a NULL list, which its routines
//! read as the empty one.
//!
//! ## The repository
//!
//! A list keeps the repository it was built for (`source.repo`), without a reference:
//! applying it hands the repository to each filter, and libgit2's `crlf` filter reads
//! its configuration through it. So a list borrows its repository: `'r` is that
//! borrow, carried by every owner and handle, as `Refdb<'r>` carries its own, and the
//! repository cannot be dropped, nor reached exclusively, while the list lives.
//! Freeing a list does not touch the repository.
//!
//! The list also keeps a pointer to each filter it holds, which stays in libgit2's
//! filter registry until `git_filter_unregister`: the wrapper of that routine must not
//! let a filter go while a list holds it, since the list's free calls its `cleanup`.
//! The constructors pin libgit2's runtime first (`runtime_pinned`), so the registry and
//! its built-in filters outlive every list.
//!
//! ## Threads
//!
//! A list is neither `Send` nor `Sync`: applying it reaches its repository, whose
//! caches libgit2 fills unsynchronised, and its filters update their state in place.
//!
//! libgit2 marks a registered filter initialised the first time a list selects it
//! (`filter_initialize`, writing `fdef->initialized` under the registry's *read* lock,
//! or none from `git_filter_list_push`), so two threads loading their first lists at
//! once race on that flag, whatever repositories they use. The constructors here close
//! that race among themselves for libgit2's built-in filters, `crlf` and `ident`: the
//! first of them initialises both, once, before it loads anything, and the others wait
//! for it. libgit2's other routines that load a list internally
//! (`git_repository_hashfile`, `git_index_add_bypath`, checkout, ...) do not go through
//! that, so their first loads still race with each other and with this module's first
//! one: libgit2's defect, which only initialising the filters when the runtime is pinned
//! would close for the whole crate. A filter registered later through
//! `git2/sys/filter.h` is initialised by the first load that selects it: the wrapper of
//! `git_filter_register` must settle that.
//!
//! ## Raw seams
//!
//! The direction is a raw `git_filter_mode_t` and the flags a raw `git_filter_flag_t`
//! set, until those are wrapped: libgit2 only compares the one and tests bits of the
//! other, so every value is accepted. [`FilterList::load_ext`] takes the options as the
//! raw `git_filter_options`, which it only copies. The blob the forms ending in
//! `_with_blob` take is an object handle of type blob (an
//! [`ObjectRef`](crate::libgit2::src::libgit2::object::ObjectRef), which a blob is); an object
//! of another type is refused before libgit2 reads it as a blob.
//!
//! ## Paths
//!
//! A list is loaded for a path relative to the working directory. libgit2 checks that
//! with an assertion in its attribute lookup, which every load reaches (the built-in
//! filters select by attribute, and cannot be unregistered): a release build fails the
//! load with `GIT_ERROR_INTERNAL`, but a debug build (`_DEBUG`) aborts the process. The
//! loading constructors therefore refuse an absolute path themselves, with the error a
//! release build reports, without calling libgit2.
//!
//! The constructors can fail and set the calling thread's libgit2 error, which they
//! return, copied out, as an [`ErrorBuf`].

use core::ffi::{CStr, c_char, c_int};
use core::fmt;
use core::hash::Hash;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ptr::{self, NonNull};
use std::ffi::CString;
use std::sync::Once;

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::src::libgit2::object::ObjectRef;
use crate::libgit2::src::libgit2::repository::RepositoryRef;
use crate::libgit2::src::util::errors::{
    ErrorBuf, ErrorClass, ErrorCode, last, runtime_pinned, set,
};

define_ctype!(
    /// A stream that data is written into piece by piece, then closed
    /// (`git_writestream`): the sink of libgit2's filters, and the writer
    /// `git_blob_create_from_stream` opens.
    ///
    /// Only ever behind a pointer, in the container its producer embedded it in (see
    /// the [module docs](self#write-streams)): owned as a [`WriteStreamBox`] or a
    /// [`RustWriteStream`], borrowed as a [`WriteStreamRef`] / [`WriteStreamMut`].
    /// There is no public constructor of a bare header, and no setter: replacing a
    /// member would detach it from the container its functions cast the header to.
    ///
    /// Besides `from_ptr`'s requirements, building a handle requires the pointer to be
    /// a header in place in its container, its three members set to that container's
    /// functions, and the stream to accept writes for the handle's lifetime: not
    /// closed, unless it is a [`RustWriteStream`]'s, whose members accept any order of
    /// calls. Lending a handle to C code that closes the stream (as
    /// `git_filter_list_stream_*` do with their target) is sound only for such a
    /// stream, or when the handle is not written through again.
    ///
    /// The handles are neither `Send` nor `Sync`: a C stream may hold a repository or
    /// a filter session that belongs to its thread.
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::src::libgit2::filter::WriteStreamBox;
    ///
    /// fn send<T: Send>(_: T) {}
    /// fn forward(stream: WriteStreamBox) {
    ///     send(stream);
    /// }
    /// ```
    ///
    /// Wraps: git_writestream
    WriteStream,
    WriteStreamRef,
    WriteStreamMut,
    ffi::git_writestream
);

impl WriteStream {
    /// Closes the stream, then frees it: its `close` member, then its `free` member.
    ///
    /// Closing ends a stream, so the owner is consumed and freed whether or not
    /// closing fails. A negative return is an error, which comes back with the thread's
    /// last libgit2 error; any other return is success, as libgit2's callers read it.
    ///
    /// Not every stream is meant to be closed by its owner: the stream
    /// `git_blob_create_from_stream` opens is committed instead, and closing it
    /// discards what was written (dropping it does the same).
    ///
    /// Field: `git_writestream.close`
    ///
    /// # Errors
    ///
    /// A negative return from `close` is an error, which comes back with the thread's last libgit2
    /// error; the stream is freed either way.
    pub fn close(mut stream: WriteStreamBox) -> Result<(), StreamError> {
        // SAFETY: `stream` is dropped right below, so the stream is freed and never
        // written or closed again.
        let closed = unsafe { stream.as_mut().close() };
        drop(stream);
        closed
    }
}

impl WriteStreamMut<'_> {
    /// Writes `data` into the stream: its `write` member, given the slice's pointer
    /// and length. The stream reads the bytes during the call only.
    ///
    /// # Errors
    ///
    /// A negative return is an error, which comes back with the thread's last libgit2
    /// error; any other return is success, as libgit2's callers read it.
    ///
    /// # Panics
    ///
    /// If the stream has no `write` member, which no producer leaves unset and the
    /// handle's invariant excludes.
    ///
    /// Field: `git_writestream.write`
    pub fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
        let stream = self.as_mut_ptr();
        // SAFETY: `stream` is the handle's pointer to the live object it borrows; the projection
        // forms no reference to it.
        let write_ptr = unsafe { &raw const (*stream).write };
        // SAFETY: a read through the exclusive handle's pointer to a live header.
        let write = unsafe { write_ptr.read() };
        let write = write.expect("a git_writestream always has a write member");
        // SAFETY: per the handle's invariant, `write` is the member the stream's
        // producer set for this header, in place in its container, and the stream
        // accepts writes; it reads `data.len()` bytes at `data` during the call only.
        // The handle is exclusive, so no other call on the stream runs meanwhile.
        let rc = unsafe { write(stream, data.as_ptr().cast(), data.len()) };
        check(rc)
    }

    /// Closes the stream: its `close` member. Results read as [`write`](Self::write)'s.
    ///
    /// # Panics
    ///
    /// If the stream has no `close` member, which the handle's invariant excludes.
    ///
    /// # Safety
    ///
    /// Closing ends the stream: afterwards it must not be written or closed again
    /// through any handle, only freed, unless it is a [`RustWriteStream`]'s.
    ///
    /// Field: `git_writestream.close`
    pub(crate) unsafe fn close(&mut self) -> Result<(), StreamError> {
        let stream = self.as_mut_ptr();
        // SAFETY: `stream` is the handle's pointer to the live object it borrows; the projection
        // forms no reference to it.
        let close_ptr = unsafe { &raw const (*stream).close };
        // SAFETY: a read through the exclusive handle's pointer to a live header.
        let close = unsafe { close_ptr.read() };
        let close = close.expect("a git_writestream always has a close member");
        // SAFETY: as in `write`, for the `close` member; the caller ends the stream.
        let rc = unsafe { close(stream) };
        check(rc)
    }
}

/// A stream member's return as a result: negative is an error, described by the
/// thread's last libgit2 error, which is copied before any other libgit2 call.
fn check(rc: c_int) -> Result<(), StreamError> {
    if rc < 0 {
        Err(StreamError::from_libgit2(rc))
    } else {
        Ok(())
    }
}

/// `git_writestream.free` in `impl_cdrop!`'s shape: the stream's own destructor.
///
/// A stream without one is left alone: no producer leaves the member unset.
///
/// # Safety
///
/// `stream` is a live header in its container, with its members set, which the caller
/// owns and does not use again.
pub(crate) unsafe fn stream_free(stream: *mut ffi::git_writestream) {
    // SAFETY: per this function's contract the caller owns the live header; the projection forms no
    // reference to it.
    let free_ptr = unsafe { &raw const (*stream).free };
    // SAFETY: reads the initialised member.
    let free = unsafe { free_ptr.read() };
    if let Some(free) = free {
        // SAFETY: `free` is the member the stream's producer set for this header; it
        // releases the container, which the caller gives up.
        unsafe { free(stream) }
    }
}

/// Releases a stream through its own `free` member, which frees the container the
/// header is embedded in: a blob writer's file buffer, hint path and allocation; a
/// [`RustWriteStream`]'s box. Stateless: the routine is recovered from the object.
///
/// Field: `git_writestream.free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteStreamFree;

impl_cdrop!(WriteStreamFree, WriteStream, stream_free);

/// The sole owner of a stream, C's or Rust's, released through its `free` member.
///
/// [`RustWriteStream::into_box`] builds one safely; a stream libgit2 opens is adopted
/// by the wrapper of the routine that opens it.
pub type WriteStreamBox = CBox<WriteStream, WriteStreamFree>;

/// What a stream implemented in Rust does when its `write` and `close` members are
/// called, by libgit2 or through a handle; [`RustWriteStream`] adapts it.
///
/// libgit2 writes any number of times and then closes once, but a stream lent to other
/// code may see any order, so an implementation must not rely on one. An error is
/// returned to the caller as its code and becomes the thread's last libgit2 error. A
/// panic aborts the process, since it cannot unwind into C.
///
/// Handwritten
pub trait StreamWriter {
    /// Takes the next piece of the stream's data.
    ///
    /// # Errors
    ///
    /// An error ends the producer's write: libgit2 reports the failure of the operation that
    /// streams into it.
    fn write(&mut self, data: &[u8]) -> Result<(), StreamError>;

    /// Ends the stream's data. The default accepts.
    ///
    /// # Errors
    ///
    /// An error fails the operation that streams into it, after the data was written.
    fn close(&mut self) -> Result<(), StreamError> {
        Ok(())
    }
}

/// Collects the data written.
impl StreamWriter for Vec<u8> {
    fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
        self.extend_from_slice(data);
        Ok(())
    }
}

/// Forwards to the borrowed writer, which the caller keeps.
impl<T: StreamWriter + ?Sized> StreamWriter for &mut T {
    fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
        (**self).write(data)
    }

    fn close(&mut self) -> Result<(), StreamError> {
        (**self).close()
    }
}

/// A stream's header and the writer its members run, boxed together: the header first,
/// so a pointer to it is a pointer to the box.
#[repr(C)]
pub(crate) struct Repr<W> {
    stream: ffi::git_writestream,
    writer: W,
}

/// A stream implemented in Rust: a `git_writestream` whose members run a
/// [`StreamWriter`], owned by Rust.
///
/// It lends [`WriteStreamRef`] / [`WriteStreamMut`] handles, as any owner does, and the
/// writer itself, through [`writer`](Self::writer) / [`writer_mut`](Self::writer_mut)
/// or [`into_inner`](Self::into_inner) once the stream is done. Its members accept any
/// order of calls, so it may be lent to C code that closes it and written again
/// afterwards. [`into_box`](Self::into_box) erases the writer's type into a
/// [`WriteStreamBox`], for code that takes any stream (a filter's `stream` callback
/// hands one to libgit2, which frees it); dropping either drops the writer.
///
/// ```
/// use libgit2::libgit2::src::libgit2::filter::RustWriteStream;
///
/// let mut stream = RustWriteStream::new(Vec::new());
/// stream.as_mut().write(b"hello, ").unwrap();
/// stream.as_mut().write(b"world").unwrap();
/// assert_eq!(stream.into_inner(), b"hello, world");
/// ```
///
/// A handle borrows the stream, and an erased one cannot borrow anything:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::src::libgit2::filter::RustWriteStream;
///
/// let mut out = Vec::new();
/// let stream = RustWriteStream::new(&mut out);
/// let _boxed = stream.into_box();
/// ```
///
/// It crosses threads only with a writer that does:
///
/// ```compile_fail,E0277
/// use std::rc::Rc;
///
/// use {libgit2::libgit2::src::libgit2::filter::RustWriteStream, libgit2::libgit2::src::libgit2::filter::StreamError, libgit2::libgit2::src::libgit2::filter::StreamWriter};
///
/// struct Shared(Rc<()>);
///
/// impl StreamWriter for Shared {
///     fn write(&mut self, _: &[u8]) -> Result<(), StreamError> {
///         Ok(())
///     }
/// }
///
/// let stream = RustWriteStream::new(Shared(Rc::new(())));
/// std::thread::spawn(move || drop(stream));
/// ```
///
/// Wraps: `git_writestream`
/// Handwritten
pub struct RustWriteStream<W: StreamWriter> {
    pub(crate) raw: NonNull<Repr<W>>,
    _owns: PhantomData<Box<Repr<W>>>,
}

// SAFETY: a `RustWriteStream` uniquely owns its box, as `Box<Repr<W>>` does: the header
// holds three function pointers fixed at construction, nothing else points into the
// box while the owner holds it, and no member call outlives the borrow that made it.
// Moving the owner to another thread moves `W` and three code addresses.
unsafe impl<W: StreamWriter + Send> Send for RustWriteStream<W> {}

// SAFETY: `&RustWriteStream` reaches `&W` ([`RustWriteStream::writer`]) and a shared
// handle, which neither calls a member nor writes the header; every call and write
// needs `&mut`.
unsafe impl<W: StreamWriter + Sync> Sync for RustWriteStream<W> {}

impl<W: StreamWriter> RustWriteStream<W> {
    /// A stream whose members run `writer`.
    pub fn new(writer: W) -> Self {
        let repr = Box::new(Repr {
            stream: ffi::git_writestream {
                write: Some(write_with::<W>),
                close: Some(close_with::<W>),
                free: Some(free_with::<W>),
            },
            writer,
        });
        Self {
            raw: NonNull::from(Box::leak(repr)),
            _owns: PhantomData,
        }
    }

    /// The stream's header.
    pub(crate) fn header(&self) -> *mut ffi::git_writestream {
        // The header is `Repr`'s first field, at offset 0 (`#[repr(C)]`).
        self.raw.as_ptr().cast()
    }

    /// Shared handle to the stream.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a box is never NULL"
    )]
    pub fn as_ref(&self) -> WriteStreamRef<'_> {
        // SAFETY: the header of the live box this owner holds, in place in its `Repr`
        // with every member set to `Repr<W>`'s trampolines; shared for `&self`.
        unsafe { WriteStreamRef::from_ptr(self.header()) }.expect("a box is never NULL")
    }

    /// Exclusive handle to the stream, to write into it or lend it to code that does.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a box is never NULL"
    )]
    pub fn as_mut(&mut self) -> WriteStreamMut<'_> {
        // SAFETY: as in `as_ref`, exclusive for `&mut self`; the trampolines accept any
        // order of calls.
        unsafe { WriteStreamMut::from_ptr(self.header()) }.expect("a box is never NULL")
    }

    /// The writer.
    #[must_use]
    pub fn writer(&self) -> &W {
        // SAFETY: the writer of the live box this owner holds; no member call runs while `&self`
        // lives, since each needs a handle borrowed from `&mut self`.
        unsafe { &(*self.raw.as_ptr()).writer }
    }

    /// The writer, exclusively.
    pub fn writer_mut(&mut self) -> &mut W {
        // SAFETY: as in `writer`, exclusive for `&mut self`.
        unsafe { &mut (*self.raw.as_ptr()).writer }
    }

    /// The writer, the stream's header released.
    #[must_use]
    pub fn into_inner(self) -> W {
        let this = ManuallyDrop::new(self);
        // SAFETY: the box this owner held, from `Box::leak` in `new`; `this` is never
        // dropped, so the box is reclaimed once.
        let repr = unsafe { Box::from_raw(this.raw.as_ptr()) };
        repr.writer
    }

    /// The stream as an owner of any stream: it is freed through its `free` member,
    /// which drops the writer and the box.
    ///
    /// `WriteStreamBox` carries no lifetime, so the writer must borrow nothing.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a box is never NULL"
    )]
    pub fn into_box(self) -> WriteStreamBox
    where
        W: 'static,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: the header of the live box this owner held, which it gives up (`this`
        // is never dropped); its `free` member, `free_with::<W>`, reclaims exactly that
        // box, and `W: 'static` outlives any owner.
        unsafe { WriteStreamBox::from_c(this.header()) }.expect("a box is never NULL")
    }
}

impl<W: StreamWriter> Drop for RustWriteStream<W> {
    fn drop(&mut self) {
        // SAFETY: the box this owner holds, from `Box::leak` in `new`, reclaimed once.
        drop(unsafe { Box::from_raw(self.raw.as_ptr()) });
    }
}

impl<W: StreamWriter + fmt::Debug> fmt::Debug for RustWriteStream<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustWriteStream")
            .field("writer", self.writer())
            .finish()
    }
}

/// The writer of the box `stream` heads.
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>`, which no other reference reaches for
/// `'a`.
unsafe fn writer_of<'a, W>(stream: *mut ffi::git_writestream) -> &'a mut W {
    // SAFETY: per this function's contract; the header is at offset 0 of its `Repr`.
    unsafe { &mut (*stream.cast::<Repr<W>>()).writer }
}

/// A writer's result as the member's return: 0, or the error's code once it is the
/// thread's last libgit2 error.
fn returned(result: Result<(), StreamError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => error.raise(),
    }
}

/// `git_writestream.write` over [`StreamWriter::write`].
///
/// A NULL `buffer` is empty data when `len` is 0, and refused otherwise.
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>`, which nothing else reaches during the
/// call; `buffer` is NULL or valid for reads of `len` bytes, unchanged during the call.
pub(crate) unsafe extern "C" fn write_with<W: StreamWriter>(
    stream: *mut ffi::git_writestream,
    buffer: *const c_char,
    len: usize,
) -> c_int {
    let data: &[u8] = match (buffer.is_null(), len) {
        (_, 0) => &[],
        (true, _) => {
            let error = StreamError::new(ErrorClass::Invalid, c"NULL buffer written to stream");
            return error.raise();
        }
        // SAFETY: per this function's contract, `buffer` is valid for `len` bytes.
        (false, _) => unsafe { core::slice::from_raw_parts(buffer.cast(), len) },
    };
    // SAFETY: per this function's contract.
    let writer = unsafe { writer_of::<W>(stream) };
    returned(writer.write(data))
}

/// `git_writestream.close` over [`StreamWriter::close`].
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>`, which nothing else reaches during the
/// call.
unsafe extern "C" fn close_with<W: StreamWriter>(stream: *mut ffi::git_writestream) -> c_int {
    // SAFETY: per this function's contract.
    let writer = unsafe { writer_of::<W>(stream) };
    returned(writer.close())
}

/// `git_writestream.free`: drops the `Repr<W>` box, writer and all.
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>` box, which the caller owns and does not
/// use again.
unsafe extern "C" fn free_with<W: StreamWriter>(stream: *mut ffi::git_writestream) {
    // SAFETY: per this function's contract, the box from `RustWriteStream::new`, given
    // up by its owner.
    drop(unsafe { Box::from_raw(stream.cast::<Repr<W>>()) });
}

/// A stream's failure: the code its member returned, and the error it left as the
/// thread's last libgit2 error, copied out; or the failure a [`StreamWriter`] reports.
///
/// A writer's error reaches its caller the way C's do: its member returns the code,
/// and the class and message become the thread's last error.
///
/// Handwritten
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct StreamError {
    pub(crate) code: c_int,
    pub(crate) class: c_int,
    pub(crate) message: CString,
}

impl StreamError {
    /// A failure in `class` described by `message`, returned as `GIT_ERROR`.
    #[must_use]
    pub fn new(class: ErrorClass, message: &CStr) -> Self {
        Self {
            code: ErrorCode::Generic.raw(),
            class: class.raw(),
            message: message.to_owned(),
        }
    }

    /// The same failure, returned as `code` (`GIT_EUSER`, say). [`ErrorCode::Ok`]
    /// would read as success, so the code then stays `GIT_ERROR`.
    #[must_use]
    pub fn with_code(mut self, code: ErrorCode) -> Self {
        if code != ErrorCode::Ok {
            self.code = code.raw();
        }
        self
    }

    /// The returned code; `None` for one the header does not name, which a stream may
    /// return ([`raw_code`](Self::raw_code) has it).
    #[must_use]
    pub fn code(&self) -> Option<ErrorCode> {
        ErrorCode::from_raw(self.code)
    }

    /// The returned code, always negative.
    #[must_use]
    pub fn raw_code(&self) -> c_int {
        self.code
    }

    /// The error's class; `None` for a class the header does not name
    /// ([`raw_class`](Self::raw_class) has it).
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        ErrorClass::from_raw(self.class)
    }

    /// The error's class as libgit2 stores it.
    #[must_use]
    pub fn raw_class(&self) -> c_int {
        self.class
    }

    /// The error message.
    #[must_use]
    pub fn message(&self) -> &CStr {
        &self.message
    }

    /// The failure a member reported by returning `code`, with the thread's last error.
    fn from_libgit2(code: c_int) -> Self {
        let error = last();
        Self {
            code,
            class: error.raw_class(),
            message: error.message().to_owned(),
        }
    }

    /// Makes this the thread's last libgit2 error (`git_error_set_str`), for a member to
    /// return its code. When libgit2 cannot be initialised the error is not stored, and
    /// when the thread's error buffer is out of memory libgit2 keeps the previous one;
    /// the code is returned either way.
    pub(crate) fn raise(&self) -> c_int {
        if runtime_pinned() {
            // SAFETY: `message` is a live C string, which libgit2 copies; any class is
            // accepted. The runtime is pinned, so the thread state is looked up under
            // libgit2's live TLS key.
            let _ = unsafe { ffi::git_error_set_str(self.class, self.message.as_ptr()) };
        }
        self.code
    }
}

/// A failure reported by another libgit2 wrapper, returned as `GIT_ERROR`.
impl From<ErrorBuf> for StreamError {
    fn from(error: ErrorBuf) -> Self {
        Self {
            code: ErrorCode::Generic.raw(),
            class: error.raw_class(),
            message: error.message().to_owned(),
        }
    }
}

impl fmt::Debug for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("StreamError");
        match self.code() {
            Some(code) => s.field("code", &code),
            None => s.field("code", &self.code),
        };
        match self.class() {
            Some(class) => s.field("class", &class),
            None => s.field("class", &self.class),
        };
        s.field("message", &self.message).finish()
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message.to_string_lossy())
    }
}

impl std::error::Error for StreamError {}

/// A list of filters to apply to one path (`git_filter_list`), built for the
/// repository it borrows for `'r`.
///
/// libgit2 allocates and frees it, so it is only ever reached through a
/// [`FilterListBox`] and its handles. It is opaque: no field is published. See the
/// [module docs](self#filter-lists) for its ownership and its tie to the repository.
///
/// A list has no copy:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::src::libgit2::filter::FilterListBox;
///
/// fn copy(list: FilterListBox<'_>) {
///     let _copy = list.clone();
/// }
/// ```
///
/// The exclusive handle is invariant in `'r`, as `&mut` is in its pointee:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::filter::FilterListMut;
///
/// fn shrink<'a, 'r>(m: FilterListMut<'a, 'static>) -> FilterListMut<'a, 'r> {
///     m
/// }
/// ```
///
/// An owner cannot outlive the repository it borrows:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::filter::FilterListBox;
///
/// fn keep<'r>(list: FilterListBox<'r>) -> FilterListBox<'static> {
///     list
/// }
/// ```
///
/// A handle cannot outlive its owner, nor alias an exclusive one:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::src::libgit2::filter::{FilterListBox, FilterListRef};
///
/// fn escape(make: impl Fn() -> FilterListBox<'static>) {
///     let r: FilterListRef<'_, 'static>;
///     {
///         let list = make();
///         r = list.as_ref();
///     }
///     let _ = r;
/// }
/// ```
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::src::libgit2::filter::FilterListBox;
///
/// fn alias(mut list: FilterListBox<'static>) {
///     let a = list.as_mut();
///     let b = list.as_mut();
///     drop((a, b));
/// }
/// ```
///
/// No list crosses threads:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::filter::FilterListBox;
///
/// fn send<T: Send>(_: T) {}
/// fn check(list: FilterListBox<'static>) {
///     send(list);
/// }
/// ```
///
/// Wraps: `git_filter_list`
/// Handwritten
#[repr(transparent)]
pub struct FilterList<'r>(
    ffi::git_filter_list,
    PhantomData<(*const (), &'r ffi::git_repository)>,
);

/// Shared borrow of a [`FilterList`]. `Copy`, like `&T`.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct FilterListRef<'a, 'r>(CBorrowedPtr<'a, FilterList<'r>>);

/// Exclusive borrow of a [`FilterList`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// Handwritten
#[repr(transparent)]
pub struct FilterListMut<'a, 'r>(
    FilterListRef<'a, 'r>,
    // Invariant in the list, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut FilterList<'r>>,
);

// SAFETY: `FilterList` is `#[repr(transparent)]` over `git_filter_list` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, FilterList<'r>>` (`FilterListMut` through `FilterListRef`) with
// zero-sized markers and no `Drop`, hand out no reference to the list, and
// `FilterListRef` has no write operation. `FilterListMut` is invariant in the list
// through its `PhantomData<&'a mut FilterList<'r>>`.
unsafe impl<'r> CCell for FilterList<'r> {
    type C = ffi::git_filter_list;
    type Mut<'a>
        = FilterListMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = FilterListRef<'a, 'r>
    where
        Self: 'a;
}

impl<'r> FilterListRef<'_, 'r> {
    /// Borrows the list at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live filter list, built for a repository that
    /// outlives `'r`, which stays live for `'a` and is not written through another
    /// handle meanwhile.
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_filter_list) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_filter_list` that outlives `'a` and is not written meanwhile.
        unsafe { <FilterList<'r> as CCell>::ref_from_ptr(ptr) }
    }

    /// Read-only pointer to the C object.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_filter_list {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> FilterListMut<'_, 'r> {
    /// Borrows the list at `ptr` exclusively; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// As [`FilterListRef::from_ptr`], plus: nothing else reaches the list while the
    /// result lives.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_filter_list) -> Option<Self> {
        // SAFETY: the caller upholds `FilterListRef::from_ptr`'s contract and
        // exclusivity.
        unsafe { FilterListRef::from_ptr(ptr) }.map(|r| Self(r, PhantomData))
    }

    /// Writable pointer to the C object.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_filter_list {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> FilterListRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> FilterListMut<'_, 'r> {
        FilterListMut(self.0, PhantomData)
    }
}

impl fmt::Debug for FilterListRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilterList").finish_non_exhaustive()
    }
}

impl fmt::Debug for FilterListMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases a filter list through `git_filter_list_free`: each filter's `cleanup`
/// gets the state it keeps for the list, then the entry array and the list are freed.
///
/// The free reads neither the repository nor the path; it reaches each filter the list
/// holds, which the registry keeps (see the [module docs](self#the-repository)).
///
/// Wraps: `git_filter_list_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct FilterListFree;

// SAFETY: `git_filter_list_free` releases a live list exactly once, with every state
// it holds: each entry's payload through its filter's `cleanup`, the entry array and
// the block, all from libgit2's allocator. It needs no state beyond the pointer, and no
// repository.
unsafe impl<'r> CDrop<FilterList<'r>> for FilterListFree {
    unsafe fn c_drop(&self, ptr: NonNull<FilterList<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live list whose ownership
        // the caller gives up; `FilterList` is transparent over `git_filter_list`. It
        // was built after the runtime was pinned, so its filters are still registered.
        unsafe { ffi::git_filter_list_free(ptr.as_ptr().cast()) }
    }
}

/// An owned [`FilterList`], released by `git_filter_list_free` on drop.
pub type FilterListBox<'r> = CBox<FilterList<'r>, FilterListFree>;

/// `Ok` for a relative `path`; for an absolute one, the error a release build of
/// libgit2 reports from the assertion in `collect_attr_files`, set as the thread's.
pub(crate) fn relative(path: &CStr) -> Result<(), ErrorBuf> {
    if path.to_bytes().first() == Some(&b'/') {
        set(
            ErrorClass::Internal,
            c"unrecoverable internal error: '!((path)[0] == '/')'",
        );
        return Err(last());
    }
    Ok(())
}

/// The list a loading constructor returning `rc` wrote, `None` for none, or libgit2's
/// error. libgit2 writes no list on failure.
fn loaded(
    rc: c_int,
    list: Option<FilterListBox<'_>>,
) -> Result<Option<FilterListBox<'_>>, ErrorBuf> {
    if rc < 0 {
        drop(list);
        return Err(last());
    }
    Ok(list)
}

/// The `git_blob` an object of type blob is, live for `'b`, the handle's borrow: what
/// the `_with_blob` forms build from their object, and nothing else does.
#[derive(Clone, Copy)]
struct LiveBlob<'b>(NonNull<ffi::git_blob>, PhantomData<&'b ffi::git_blob>);

impl<'b> LiveBlob<'b> {
    /// The blob `object` is; for an object of any other type, which libgit2 would read
    /// as a blob, a `GIT_ERROR_FILTER` error "the object is not a blob", set as the
    /// thread's.
    fn of(object: ObjectRef<'b, '_>) -> Result<Self, ErrorBuf> {
        // SAFETY: the handle addresses a live object; `git_object_type` reads its cached
        // type.
        if unsafe { ffi::git_object_type(object.as_ptr()) } != ffi::GIT_OBJECT_BLOB {
            set(ErrorClass::Filter, c"the object is not a blob");
            return Err(last());
        }
        // A blob object is a `git_blob`, whose first member is the `git_object`; the
        // handle keeps it alive for `'b`.
        let blob = NonNull::new(object.as_ptr().cast_mut().cast());
        Ok(Self(blob.expect("a handle is never NULL"), PhantomData))
    }

    /// The pointer libgit2 takes: the blob, or NULL for none.
    fn as_ptr(blob: Option<Self>) -> *mut ffi::git_blob {
        blob.map_or(ptr::null_mut(), |b| b.0.as_ptr())
    }
}

impl<'r> FilterList<'r> {
    /// An empty list for `repo`, applying in direction `mode` with `flags`, to which
    /// filters are then added (`git_filter_list_push`, `git2/sys/filter.h`). The list
    /// has no path.
    ///
    /// The list keeps `repo`, which its filters read when it is applied: it borrows
    /// `repo` for `'r`, and the repository cannot go first:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::ffi;
    /// use libgit2::libgit2::src::libgit2::filter::FilterList;
    /// use libgit2::libgit2::src::libgit2::repository::Repository;
    ///
    /// let repo = Repository::new().unwrap();
    /// let list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
    /// drop(repo);
    /// drop(list);
    /// ```
    ///
    /// Raw seam: `mode` is a raw `git_filter_mode_t` and `flags` a raw
    /// `git_filter_flag_t` set until those are wrapped; libgit2 accepts every value.
    ///
    /// Wraps: `git_filter_list_new`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's out-of-memory error, when the list cannot be allocated.
    pub fn new(
        repo: RepositoryRef<'r>,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<FilterListBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        initialise_builtin_filters();
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, left NULL on failure; `repo` a live
        // repository, which the list keeps for `'r`, the handle's borrow, on this
        // thread, which neither leaves. The runtime is pinned, so a failure is reported
        // on live thread-local state.
        let rc = unsafe {
            ffi::git_filter_list_new(&raw mut out, repo.as_ptr().cast_mut(), mode, flags)
        };
        // SAFETY: on success `out` is a fresh list nothing else references, released by
        // `git_filter_list_free` as `FilterListFree` does; on failure it is NULL. Its
        // repository outlives `'r`.
        match loaded(rc, unsafe { FilterListBox::from_c(out) })? {
            Some(list) => Ok(list),
            // A slot left NULL on success would be a libgit2 bug, reported as the
            // thread's error.
            None => Err(last()),
        }
    }

    /// The filters that apply to `path` in `repo`, in direction `mode`, as its
    /// attributes select them; `None` when none does.
    ///
    /// `path` is relative to the working directory, and need not exist; an absolute one
    /// fails with `GIT_ERROR_INTERNAL`, as a release build of libgit2 fails it (see the
    /// [module docs](self#paths)). `flags` choose
    /// where attributes are read from (`GIT_FILTER_NO_SYSTEM_ATTRIBUTES`,
    /// `GIT_FILTER_ATTRIBUTES_FROM_HEAD`) and let a filter ignore unsafe conversions
    /// (`GIT_FILTER_ALLOW_UNSAFE`); `GIT_FILTER_ATTRIBUTES_FROM_COMMIT` reads the commit
    /// only [`load_ext`](Self::load_ext) takes, here the zero id, which libgit2 skips as
    /// missing.
    ///
    /// The list keeps `repo`, as [`new`](Self::new) does:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::ffi;
    /// use libgit2::libgit2::src::libgit2::filter::FilterList;
    /// use libgit2::libgit2::src::libgit2::repository::Repository;
    ///
    /// let repo = Repository::open(c".").unwrap();
    /// let list = FilterList::load(repo.as_ref(), c"a.txt", ffi::GIT_FILTER_TO_ODB, 0);
    /// drop(repo);
    /// drop(list);
    /// ```
    ///
    /// Raw seam: `mode` and `flags` as for [`new`](Self::new).
    ///
    /// Wraps: `git_filter_list_load`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, when the attributes cannot be read (with
    /// `GIT_FILTER_ATTRIBUTES_FROM_HEAD`, when `HEAD` is unborn), or a filter's `check`
    /// fails.
    pub fn load(
        repo: RepositoryRef<'r>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        Self::load_from(repo, None, path, mode, flags)
    }

    /// [`load`](Self::load) for the content of `blob`, an object of type blob (of any
    /// repository): each filter's `check` sees the blob's id (`git_filter_source_id`)
    /// while it decides whether to apply. The list keeps neither the blob nor its id,
    /// and libgit2's built-in filters do not read it.
    ///
    /// Wraps: `git_filter_list_load`
    ///
    /// # Errors
    ///
    /// Refuses, with a `GIT_ERROR_FILTER` error "the object is not a blob", an object
    /// of another type, which libgit2 would read as a blob.
    pub fn load_with_blob(
        repo: RepositoryRef<'r>,
        blob: ObjectRef<'_, '_>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        Self::load_from(repo, Some(blob), path, mode, flags)
    }

    /// `git_filter_list_load`, with no blob or an object that must be one.
    fn load_from(
        repo: RepositoryRef<'r>,
        blob: Option<ObjectRef<'_, '_>>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        flags: u32,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        relative(path)?;
        let blob = blob.map(LiveBlob::of).transpose()?;
        initialise_builtin_filters();
        let mut out = ptr::null_mut();
        let (repo, path) = (repo.as_ptr().cast_mut(), path.as_ptr());
        let blob = LiveBlob::as_ptr(blob);
        // SAFETY: `out` is a writable slot, left NULL unless a list is built; `repo` a
        // live repository, whose attribute caches libgit2 fills through this shared
        // handle on this thread only, and which the list keeps for `'r`; `blob` NULL or
        // live for the call, as a `LiveBlob` is, its id read for the filters' checks;
        // `path` NUL-terminated, copied. The runtime is pinned: the filter registry and its lock
        // exist, and a failure is reported on live thread-local state.
        let rc = unsafe { ffi::git_filter_list_load(&raw mut out, repo, blob, path, mode, flags) };
        // SAFETY: `out` is NULL (no filter applies, or a failure), or a fresh list
        // nothing else references, released by `git_filter_list_free` as
        // `FilterListFree` does. Its repository outlives `'r`.
        loaded(rc, unsafe { FilterListBox::from_c(out) })
    }

    /// [`load`](Self::load) with options: their `flags`, and the commit whose
    /// `.gitattributes` `GIT_FILTER_ATTRIBUTES_FROM_COMMIT` reads (`attr_commit_id`);
    /// `None` uses the defaults, no flags. libgit2 copies the options, and reads neither
    /// their `version` nor their `reserved` pointer.
    ///
    /// Raw seam: `opts` is the raw `git_filter_options` until it is wrapped, and `mode`
    /// as for [`new`](Self::new).
    ///
    /// Wraps: `git_filter_list_load_ext`
    ///
    /// # Errors
    ///
    /// Fails as [`load`](Self::load) does.
    pub fn load_ext(
        repo: RepositoryRef<'r>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        opts: Option<&ffi::git_filter_options>,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        Self::load_ext_from(repo, None, path, mode, opts)
    }

    /// [`load_ext`](Self::load_ext) for the content of `blob`, as
    /// [`load_with_blob`](Self::load_with_blob) is for [`load`](Self::load): an object
    /// of type blob, refused otherwise.
    ///
    /// Wraps: `git_filter_list_load_ext`
    ///
    /// # Errors
    ///
    /// Fails as [`load_with_blob`](Self::load_with_blob) does.
    pub fn load_ext_with_blob(
        repo: RepositoryRef<'r>,
        blob: ObjectRef<'_, '_>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        opts: Option<&ffi::git_filter_options>,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        Self::load_ext_from(repo, Some(blob), path, mode, opts)
    }

    /// `git_filter_list_load_ext`, with no blob or an object that must be one.
    fn load_ext_from(
        repo: RepositoryRef<'r>,
        blob: Option<ObjectRef<'_, '_>>,
        path: &CStr,
        mode: ffi::git_filter_mode_t,
        opts: Option<&ffi::git_filter_options>,
    ) -> Result<Option<FilterListBox<'r>>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        relative(path)?;
        let blob = blob.map(LiveBlob::of).transpose()?;
        initialise_builtin_filters();
        let mut out = ptr::null_mut();
        let (repo, path) = (repo.as_ptr().cast_mut(), path.as_ptr());
        let blob = LiveBlob::as_ptr(blob);
        // libgit2 takes the options mutably but only copies them by value.
        let opts = opts.map_or(ptr::null_mut(), |o| ptr::from_ref(o).cast_mut());
        // SAFETY: as in `load_from`; `opts` is NULL or a live `git_filter_options`, only
        // read (copied into the session) for the call: libgit2 is built with
        // `GIT_DEPRECATE_HARD`, so the `reserved` pointer is never read.
        let rc =
            unsafe { ffi::git_filter_list_load_ext(&raw mut out, repo, blob, path, mode, opts) };
        // SAFETY: as in `load_from`.
        loaded(rc, unsafe { FilterListBox::from_c(out) })
    }
}

/// Marks libgit2's built-in filters (`crlf`, `ident`) initialised, once per process:
/// pushes each onto a scratch list, which runs `filter_initialize` for it, then frees the
/// list.
///
/// libgit2 initialises a filter, unsynchronised, the first time a list selects it (see
/// the [module docs](self#threads)). Every constructor calls this, with the runtime
/// pinned, before it loads a list, as do the wrappers of routines that load one
/// internally (the blob writers of `object::blob`): the first call does the work, and
/// the others wait for it, so the flag is written before any of their loads reads it. A
/// failure (an allocation) leaves the filters to their first load, as libgit2 would.
pub(crate) fn initialise_builtin_filters() {
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        let mut list = ptr::null_mut();
        // SAFETY: `list` is a writable slot, left NULL on failure; no repository, which
        // the list only keeps, and it is never applied. The caller pinned the runtime.
        let rc = unsafe {
            ffi::git_filter_list_new(&raw mut list, ptr::null_mut(), ffi::GIT_FILTER_TO_ODB, 0)
        };
        if rc < 0 || list.is_null() {
            return;
        }
        for name in [c"crlf", c"ident"] {
            // SAFETY: a NUL-terminated name; libgit2 looks it up under its registry lock
            // and returns the registered filter, which the registry keeps, or NULL.
            let filter = unsafe { ffi::git_filter_lookup(name.as_ptr()) };
            if !filter.is_null() {
                // SAFETY: the live scratch list and a registered filter, with no payload;
                // libgit2 initialises the filter and appends an entry, or fails without
                // one. No constructor here loads meanwhile: they wait on `DONE`.
                unsafe { ffi::git_filter_list_push(list, filter, ptr::null_mut()) };
            }
        }
        // SAFETY: the scratch list, released once; its entries have no payload, which
        // the built-in filters' `cleanup` accepts.
        unsafe { ffi::git_filter_list_free(list) };
    });
}

// The layout type is the C type's size, so a list pointer converts both ways.
const _: () = assert!(size_of::<FilterList<'static>>() == size_of::<ffi::git_filter_list>());

const _: () =
    assert!(size_of::<FilterListBox<'static>>() == size_of::<*mut ffi::git_filter_list>());

#[cfg(test)]
mod unit_tests {

    use core::mem::size_of;
    use std::ffi::CString;
    use std::path::PathBuf;

    use super::*;
    use crate::libgit2::src::libgit2::repository::{Repository, RepositoryBox};

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process: libgit2 initialises a filter,
    /// unsynchronised, the first time any list selects it, and other modules' tests load
    /// lists through raw calls the crate cannot order (see the module docs).
    fn isolated(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
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

    /// A fresh repository with a working directory whose `.gitattributes` gives `*.txt`
    /// the `crlf` filter and `*.id` the `ident` one; removed on drop.
    struct Workdir {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Workdir {
        fn new(tag: &str) -> Self {
            assert!(runtime_pinned());
            let path = std::env::temp_dir()
                .join(format!("crustify-filter-unit-{}-{tag}", std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            std::fs::write(
                path.join(".gitattributes"),
                "*.txt text eol=crlf\n*.id ident\n",
            )
            .unwrap();
            Self {
                path,
                repo: Some(repo),
            }
        }

        fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }
    }

    impl Drop for Workdir {
        fn drop(&mut self) {
            drop(self.repo.take());
            drop(std::fs::remove_dir_all(&self.path));
        }
    }

    fn length(list: &FilterListBox<'_>) -> usize {
        // SAFETY: a live list; libgit2 only reads its entry count.
        unsafe { ffi::git_filter_list_length(list.as_ref().as_ptr()) }
    }

    #[test]
    fn layout_is_one_pointer() {
        assert_eq!(
            size_of::<FilterList<'_>>(),
            size_of::<ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<FilterListBox<'_>>(),
            size_of::<*mut ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<Option<FilterListBox<'_>>>(),
            size_of::<*mut ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<FilterListRef<'_, '_>>(),
            size_of::<*const ffi::git_filter_list>()
        );
        assert_eq!(
            size_of::<FilterListMut<'_, '_>>(),
            size_of::<*mut ffi::git_filter_list>()
        );
        assert_eq!(size_of::<FilterListFree>(), 0);
    }

    /// The handles over Rust storage, so Miri checks them: the list is opaque, so they
    /// only carry the pointer, reborrow and print; nothing is freed.
    #[test]
    fn handles_without_c() {
        let mut storage = 0u64;
        let raw: *mut ffi::git_filter_list = ptr::from_mut(&mut storage).cast();
        // SAFETY: NULL is refused.
        assert!(unsafe { FilterListRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: NULL is refused.
        assert!(unsafe { FilterListMut::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: `raw` addresses live storage nothing else reaches while the handle
        // lives; the opaque list is never read through it.
        let mut m = unsafe { FilterListMut::<'_, 'static>::from_ptr(raw) }.unwrap();
        assert_eq!(m.as_mut_ptr(), raw);
        assert_eq!(m.as_ref().as_ptr(), raw.cast_const());
        let mut inner = m.as_mut();
        assert_eq!(inner.as_mut_ptr(), raw);
        assert_eq!(format!("{inner:?}"), "FilterList { .. }");
        let r = m.as_ref();
        let copy = r;
        assert_eq!(format!("{r:?}"), format!("{copy:?}"));
        assert_eq!(format!("{m:?}"), "FilterList { .. }");
        // SAFETY: as above, shared.
        let shared = unsafe { FilterListRef::<'_, 'static>::from_ptr(raw) }.unwrap();
        assert_eq!(shared.as_ptr(), raw.cast_const());
        assert_eq!(storage, 0);
    }

    /// `loaded` reports the thread's error for a failing code, and passes a list or the
    /// lack of one through otherwise.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn loaded_reports_failures() {
        isolated(concat!(module_path!(), "::loaded_reports_failures"), || {
            assert!(runtime_pinned());
            set(ErrorClass::Filter, c"refused here");
            let error = loaded(-1, None).unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::Filter));
            assert_eq!(error.message(), c"refused here");
            assert!(loaded(0, None).unwrap().is_none());
            let wd = Workdir::new("loaded");
            let list = FilterList::new(wd.repo(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
            assert_eq!(length(&loaded(0, Some(list)).unwrap().unwrap()), 0);
        });
    }

    /// The corrected divergence: libgit2 asserts the path is relative, which aborts a
    /// debug build; the wrappers refuse an absolute path before calling it, with the
    /// release build's error, for every loading form, and still load relative ones.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn absolute_paths_are_refused_before_c() {
        isolated(
            concat!(module_path!(), "::absolute_paths_are_refused_before_c"),
            || {
                let wd = Workdir::new("absolute");
                let expect = |r: Result<Option<FilterListBox<'_>>, ErrorBuf>| {
                    let error = r.map(|_| ()).unwrap_err();
                    assert_eq!(error.class(), Some(ErrorClass::Internal));
                    assert_eq!(
                        error.message(),
                        c"unrecoverable internal error: '!((path)[0] == '/')'"
                    );
                };
                for path in [c"/", c"/a.txt", c"//a.id"] {
                    expect(FilterList::load(
                        wd.repo(),
                        path,
                        ffi::GIT_FILTER_TO_WORKTREE,
                        0,
                    ));
                    expect(FilterList::load_ext(
                        wd.repo(),
                        path,
                        ffi::GIT_FILTER_TO_ODB,
                        None,
                    ));
                }
                // Not absolute: a leading `.` or a `/` further in.
                for path in [c"./a.txt", c"sub/a.txt", c"a.txt/"] {
                    assert!(
                        FilterList::load(wd.repo(), path, ffi::GIT_FILTER_TO_WORKTREE, 0).is_ok()
                    );
                }
            },
        );
    }

    /// The built-in filters are initialised once, before the first load; calling it again
    /// does nothing, and lists still select them.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn builtin_filters_initialise_once() {
        isolated(
            concat!(module_path!(), "::builtin_filters_initialise_once"),
            || {
                assert!(runtime_pinned());
                initialise_builtin_filters();
                initialise_builtin_filters();
                let wd = Workdir::new("builtins");
                for path in [c"a.txt", c"a.id"] {
                    let list = FilterList::load(wd.repo(), path, ffi::GIT_FILTER_TO_WORKTREE, 0)
                        .unwrap()
                        .unwrap();
                    assert_eq!(length(&list), 1);
                }
            },
        );
    }

    /// The free policy, driven by hand and through the box, releases lists with filter
    /// state (`crlf` keeps a payload its `cleanup` frees) once each (`LSan`, `ASan`); a list
    /// given back to C with `into_c` is freed by `git_filter_list_free`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn free_policy_and_c_release_lists() {
        isolated(
            concat!(module_path!(), "::free_policy_and_c_release_lists"),
            || {
                let wd = Workdir::new("free");
                let list = FilterList::load(wd.repo(), c"a.txt", ffi::GIT_FILTER_TO_WORKTREE, 0)
                    .unwrap()
                    .unwrap();
                assert_eq!(length(&list), 1);
                let raw = list.into_c();
                // SAFETY: `raw` is the live list given up above, released once.
                unsafe {
                    FilterListFree.c_drop(NonNull::new(raw).unwrap().cast::<FilterList<'_>>());
                };

                let list = FilterList::load(wd.repo(), c"a.id", ffi::GIT_FILTER_TO_ODB, 0)
                    .unwrap()
                    .unwrap();
                let raw = list.into_c();
                // SAFETY: the live list given up above, released once by C.
                unsafe { ffi::git_filter_list_free(raw) };

                let mut out = ptr::null_mut();
                // SAFETY: a writable slot, the live repository and a NUL-terminated path.
                let rc = unsafe {
                    ffi::git_filter_list_load(
                        &raw mut out,
                        wd.repo().as_ptr().cast_mut(),
                        ptr::null_mut(),
                        c"b.txt".as_ptr(),
                        ffi::GIT_FILTER_TO_ODB,
                        0,
                    )
                };
                assert_eq!(rc, 0);
                // SAFETY: the fresh list C built for `wd`'s repository, which outlives it.
                let adopted = unsafe { FilterListBox::from_c(out) }.unwrap();
                assert_eq!(length(&adopted), 1);
                drop(adopted);
            },
        );
    }
}
