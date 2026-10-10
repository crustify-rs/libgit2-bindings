//! Safe wrappers over `include/git2/commit.h`.
//!
//! # Commit arrays
//!
//! A [`CommitArray`] is a `git_commitarray`: a run of `count` commit pointers,
//! `commits`, in one buffer of libgit2's allocator, each pointer holding one reference
//! to a `git_commit`. libgit2 never allocates the struct itself: callers hold it by
//! value, `git_repository_commit_parents` fills one (overwriting both fields without
//! releasing what they held), and `git_commitarray_dispose` releases every commit with
//! `git_commit_free`, frees the buffer with `git__free` and zeroes the struct, leaving
//! it to its holder. Rust holds it the same way, as a [`CommitArrayVal`], which
//! disposes it on drop.
//!
//! Every handle upholds one invariant, which the safe API keeps and
//! [`CommitArray::from_raw`] or a crate-internal `from_ptr` over a C-owned struct must
//! vouch for: `commits` is NULL, or a live buffer of libgit2's allocator holding at
//! least `count` pointers, each to a live `git_commit` of which the array owns one
//! reference. libgit2 leaves `commits` NULL with `count == 0` for an empty result (an
//! unborn `HEAD`, or a failure). A NULL `commits` reads as the empty array whatever
//! `count` says, and is never handed to C.
//!
//! An empty array never reaches libgit2: nothing allocated it, so nothing has
//! initialised the library on its behalf, and even `git__free(NULL)` reads the
//! allocator a concurrent first `git_libgit2_init` writes. Every other array's buffer
//! came from libgit2's allocator, whose allocation initialised the library first.
//!
//! The commits themselves are a raw seam: `git_commit` is wrapped by the `object`
//! subsystem's run, which replaces [`CommitArrayRef::commit_ptrs`] with handles and
//! adds the setters that move owned commits in and out. Until then the array is read
//! as raw pointers it keeps alive, and emptied only as a whole
//! ([`CommitArrayMut::clear`]). The array asserts neither `Send` nor `Sync`: the
//! commits it owns belong to a repository's object cache, whose thread story is that
//! run's to settle.
//!
//! An array may be dropped after the repository its commits came from:
//! `git_commit_free` touches only the commit. The commits may not be *used* past it:
//! each keeps the repository it was parsed from (`git_object.repo`, uncounted, which
//! `git_repository_free` does not clear), and `git_commit_owner`, `git_commit_tree`,
//! `git_commit_parent` and every other lookup through a commit read it. So the
//! `git_commit` run's handle must borrow the repository, as `Reference<'r>` and
//! `Odb<'r>` do, and the array that holds such handles with it.
//!
//! # Extra headers
//!
//! A [`Header`] is a `git_commit_header`: a field name and a value that
//! `git_commit_create_ext` (and `git_commit_create_from_stage`, through the same
//! options) writes into the commit it creates, after `committer` and any `encoding`, as
//! `<field> <value>\n`, each newline of the value continued by a space. libgit2 never
//! allocates, keeps or frees one: callers build an array of them for the options'
//! `extra_headers`, which the call reads and forgets. So Rust holds a header by value,
//! borrowing both strings for `'h`, and a `[Header<'h>]` is the C array, the type being
//! `#[repr(transparent)]` over the struct.
//!
//! Neither string is ever NULL: libgit2 refuses a NULL one (`GIT_ERROR_INVALID`, from
//! `format_header_field`'s argument checks), so a header has no NULL state and no
//! `Default`. libgit2 writes the field as given, without checking it for a space or a
//! newline, either of which yields a commit git parses differently; a header keeps any
//! string, as the C struct does.

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CCell, CDispose, CSlice, CVal, define_ctype};

use crate::ffi;

define_ctype!(
    /// An array of commits (`git_commitarray`): `count` commit references in one buffer
    /// of libgit2's allocator, which the struct owns, as `git_repository_commit_parents`
    /// hands one out.
    ///
    /// Held by value inside a [`CommitArrayVal`], which [`new`](Self::new) builds empty
    /// for libgit2 to fill and [`from_raw`](Self::from_raw) adopts from C; read it
    /// through the owner's [`as_ref`](CVal::as_ref) handle. Not `Clone`: it owns its
    /// commits, and libgit2 has no copy routine.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::object::commit::{CommitArray, CommitArrayVal};
    ///
    /// let array = CommitArray::new();
    /// let copy: CommitArrayVal = array.clone();
    /// ```
    ///
    /// Nor `Send`: the commits it owns belong to a repository's object cache, whose
    /// thread story the `git_commit` run settles.
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::object::commit::CommitArray;
    ///
    /// let array = CommitArray::new();
    /// std::thread::spawn(move || drop(array));
    /// ```
    ///
    /// Wraps: git_commitarray
    CommitArray, CommitArrayRef, CommitArrayMut, ffi::git_commitarray
);

/// Releases an array's commits through `git_commitarray_dispose`, which hands each to
/// `git_commit_free`, the buffer to `git__free`, and zeroes the struct, leaving the
/// struct itself, Rust's storage, alone.
///
/// An empty array (`commits` NULL) is not passed to C: it owns nothing, and the call
/// would be `git__free(NULL)`, which reads `git__allocator` without synchronisation
/// against the first `git_libgit2_init` another thread may be running; it would also
/// read a NULL buffer for a non-zero `count`. Nothing orders an empty array after that
/// initialisation, since nothing allocated for it.
///
/// Wraps: git_commitarray_dispose
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct CommitArrayDispose;

// SAFETY: `git_commitarray_dispose` releases each of the first `count` commits once,
// frees `commits` once and never the struct, which it zeroes. It accepts every state
// the safe API reaches: a buffer libgit2 filled (whose allocation initialised the
// library, so the allocator it reads is settled), adopted through `from_raw` under that
// function's contract; `clear` leaves the empty array, as `new` builds it, which owns
// nothing and is released without C.
unsafe impl CDispose<CommitArray> for CommitArrayDispose {
    #[inline]
    unsafe fn c_dispose(&self, ptr: NonNull<CommitArray>) {
        let raw: *mut ffi::git_commitarray = ptr.as_ptr().cast();
        // SAFETY: per `c_dispose`'s contract `raw` addresses a live `git_commitarray`.
        let commits = unsafe { addr_of!((*raw).commits).read() };
        if commits.is_null() {
            return;
        }
        // SAFETY: `raw` is live and, per the handles' invariant, owns `commits`, a
        // buffer from libgit2's allocator of at least `count` commit references; the
        // caller gives the value up, so each is released once.
        unsafe { ffi::git_commitarray_dispose(raw) }
    }
}

/// An owned [`CommitArray`], held inline; its commits and buffer are released by
/// `git_commitarray_dispose` on drop.
pub type CommitArrayVal = CVal<CommitArray, CommitArrayDispose>;

impl CommitArray {
    /// An empty array: no commits, nothing allocated, as `git_commitarray array = {0}`.
    /// The storage libgit2 fills a result into.
    pub fn new() -> CommitArrayVal {
        CVal::new(Self(
            ffi::git_commitarray {
                commits: ptr::null(),
                count: 0,
            },
            PhantomData,
        ))
    }

    /// Adopts a `git_commitarray` C code filled, as `git_repository_commit_parents`
    /// hands one out: its commits and buffer become the result's, released by
    /// `git_commitarray_dispose` on drop.
    ///
    /// An array whose `commits` is NULL owns nothing and reads as empty, whatever
    /// `count` holds.
    ///
    /// # Safety
    ///
    /// When `raw.commits` is not NULL, it must be a live allocation from libgit2's
    /// current allocator holding at least `raw.count` pointers, each to a live
    /// `git_commit` of which `raw` owns one reference (one `git_commit_free` releases
    /// it), all of which the caller gives up: nothing else may free the buffer or
    /// release those references, nor write the buffer while the result lives. With
    /// `raw.commits` NULL there is no requirement.
    pub unsafe fn from_raw(raw: ffi::git_commitarray) -> CommitArrayVal {
        CVal::new(Self(raw, PhantomData))
    }

    /// The raw `git_commitarray`, ownership included: C code releases it with
    /// `git_commitarray_dispose`.
    pub fn into_raw(self) -> ffi::git_commitarray {
        self.0
    }
}

impl<'a> CommitArrayRef<'a> {
    /// The number of commits; 0 without a buffer, whatever `count` says, as
    /// [`commit_ptrs`](Self::commit_ptrs) reads it.
    ///
    /// Field: git_commitarray.count
    pub fn len(&self) -> usize {
        let array = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to a live
        // `git_commitarray`; any `size_t` is a valid `usize`.
        let (commits, count) = unsafe {
            (
                addr_of!((*array).commits).read(),
                addr_of!((*array).count).read(),
            )
        };
        if commits.is_null() { 0 } else { count }
    }

    /// Whether the array holds no commits.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The commits, as the [`len`](Self::len) pointers the array holds, borrowed in
    /// place for as long as the array is; each is non-NULL, and its commit stays alive
    /// at least that long, since the array owns a reference to it.
    ///
    /// Raw seam: `git_commit` has no wrapper yet; the `object` subsystem's run replaces
    /// this with commit handles. Reading a commit through a pointer is the caller's
    /// `unsafe`, and the commit must not be released through it: the array releases
    /// its references when disposed. A routine that reaches the commit's repository
    /// (`git_commit_owner`, `git_commit_tree`, `git_commit_parent`, ...) may be given
    /// one only while the repository the array came from lives (see the
    /// [module docs](self)).
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::object::commit::CommitArray;
    ///
    /// let array = CommitArray::new();
    /// let commits = array.as_ref().commit_ptrs();
    /// drop(array);
    /// let _ = commits.len();
    /// ```
    ///
    /// Field: git_commitarray.commits
    pub fn commit_ptrs(&self) -> CSlice<'a, *mut ffi::git_commit> {
        let len = self.len();
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_commitarray`.
        let commits = unsafe { addr_of!((*self.as_ptr()).commits).read() };
        match NonNull::new(commits.cast_mut()) {
            // SAFETY: per the handles' invariant a non-NULL `commits` addresses at
            // least `count` initialised commit pointers, owned by the array, so they
            // live as long as it does, past `'a`; the run only copies them out, and
            // nothing writes the buffer while the array is shared.
            Some(commits) => unsafe { CSlice::from_raw_parts(commits, len) },
            // NULL is the empty array; a run may not start at NULL.
            // SAFETY: zero elements at an aligned dangling pointer.
            None => unsafe { CSlice::from_raw_parts(NonNull::dangling(), 0) },
        }
    }
}

impl fmt::Debug for CommitArrayRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.commit_ptrs().elems()).finish()
    }
}

impl fmt::Debug for CommitArrayMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl CommitArrayMut<'_> {
    /// Releases the commits and the buffer, leaving the empty array: what
    /// `git_commitarray_dispose` leaves, and the storage libgit2 fills again without
    /// leaking what it overwrites.
    ///
    /// Field: git_commitarray.commits
    pub fn clear(&mut self) {
        let array = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer is non-NULL and addresses a live
        // `git_commitarray` (`CommitArray` is transparent over it) that, per the
        // handles' invariant, owns what it holds, released once here; it is reset to
        // the empty array below, before anything reads it again.
        unsafe { CommitArrayDispose.c_dispose(NonNull::new_unchecked(array.cast())) };
        // SAFETY: writes the empty array through the exclusive handle's pointer to a
        // live `git_commitarray`; C has zeroed it already unless `commits` was NULL, in
        // which case only `count` may differ.
        unsafe {
            addr_of_mut!((*array).commits).write(ptr::null());
            addr_of_mut!((*array).count).write(0);
        }
    }
}

// A `CommitArray` is a `git_commitarray`, so C reads and writes a value Rust holds in
// place, and the owner adds nothing to it.
const _: () = assert!(
    size_of::<CommitArray>() == size_of::<ffi::git_commitarray>()
        && align_of::<CommitArray>() == align_of::<ffi::git_commitarray>()
        && size_of::<CommitArrayVal>() == size_of::<ffi::git_commitarray>()
);

/// One extra header of a commit being created (`git_commit_header`): a field name and
/// its value, both borrowed for `'h`, as the options of `git_commit_create_ext` take
/// them in an array.
///
/// Built by [`new`](Self::new); read it through [`as_ref`](Self::as_ref) and change it
/// through [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::object::commit::Header;
///
/// let mut header = Header::new(c"line_one", c"First extra header");
/// assert_eq!(header.as_ref().field(), c"line_one");
/// header.as_mut().set_value(c"Second\nline");
/// assert_eq!(header.as_ref().value(), c"Second\nline");
///
/// // A `const` array is the C array the options point at.
/// const HEADERS: [Header<'static>; 2] = [
///     Header::new(c"line_one", c"First extra header"),
///     Header::new(c"line_two", c"Second extra header"),
/// ];
/// assert_eq!(HEADERS[1].as_ref().field(), c"line_two");
/// ```
///
/// `#[repr(transparent)]` over `git_commit_header`, so a pointer to it, or to the first
/// of a `[Header]`, is a pointer to the C struct or array, which C reads in place. `'h`
/// is the lifetime of the strings it points at, so a header cannot outlive them:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::object::commit::Header;
/// use std::ffi::CString;
///
/// let header;
/// {
///     let value = CString::new("Extra").unwrap();
///     header = Header::new(c"field", &value);
/// }
/// let _ = header.as_ref().value();
/// ```
///
/// The exclusive handle is invariant in `'h`, so one cannot be shrunk to store a
/// shorter-lived string than its header was built for:
///
/// ```compile_fail
/// use libgit2::libgit2::object::commit::HeaderMut;
///
/// fn shrink<'a, 'h>(m: HeaderMut<'a, 'static>) -> HeaderMut<'a, 'h> {
///     m
/// }
/// ```
///
/// The shared handle only reads:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::object::commit::Header;
///
/// let header = Header::new(c"field", c"value");
/// header.as_ref().set_value(c"other");
/// ```
///
/// `Copy`, `Send` and `Sync`, as a pair of `&'h CStr`s is: libgit2 never writes
/// through the strings, and only the exclusive handle writes the fields. Equal when
/// both strings are.
///
/// Wraps: git_commit_header
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Header<'h>(ffi::git_commit_header, PhantomData<&'h CStr>);

/// Shared borrow of a [`Header`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct HeaderRef<'a, 'h>(CBorrowedPtr<'a, Header<'h>>);

/// Exclusive borrow of a [`Header`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct HeaderMut<'a, 'h>(
    HeaderRef<'a, 'h>,
    // Invariant in the header, and so in `'h`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Header<'h>>,
);

// SAFETY: `Header` is `#[repr(transparent)]` over `git_commit_header` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Header<'h>>`
// (`HeaderMut` through `HeaderRef`) with zero-sized markers and no `Drop`, hand out no
// reference to the header, and `HeaderRef` has no write operation. `HeaderMut` is
// invariant in the header through its `PhantomData<&'a mut Header<'h>>`.
unsafe impl<'h> CCell for Header<'h> {
    type C = ffi::git_commit_header;
    type Ref<'a>
        = HeaderRef<'a, 'h>
    where
        Self: 'a;
    type Mut<'a>
        = HeaderMut<'a, 'h>
    where
        Self: 'a;
}

// SAFETY: a header is two string pointers that only the setters write, each a
// `&'h CStr`'s pointer: shared, immutable borrows of `CStr`, which is `Sync`, so they
// may be read from any thread. Nothing in the struct has thread affinity or interior
// mutability, and nothing is released on drop.
unsafe impl Send for Header<'_> {}
// SAFETY: as above; `HeaderMut` is an exclusive borrow of such a value, as
// `&mut Header`.
unsafe impl Send for HeaderMut<'_, '_> {}

// SAFETY: as above, and shared access only reads the fields (every write goes through
// `HeaderMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for Header<'_> {}
// SAFETY: as above; `HeaderRef` is a shared borrow, as `&Header`.
unsafe impl Send for HeaderRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for HeaderRef<'_, '_> {}
// SAFETY: `&HeaderMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for HeaderMut<'_, '_> {}

impl<'h> Header<'h> {
    /// The header `<field> <value>`. libgit2 writes both as given, each newline of
    /// `value` continued by a space; `field` should hold neither a space nor a newline.
    pub const fn new(field: &'h CStr, value: &'h CStr) -> Self {
        Self(
            ffi::git_commit_header {
                field: field.as_ptr(),
                value: value.as_ptr(),
            },
            PhantomData,
        )
    }

    /// Shared handle to the header — the getters.
    pub fn as_ref(&self) -> HeaderRef<'_, 'h> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        HeaderRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the header — the setters.
    pub fn as_mut(&mut self) -> HeaderMut<'_, 'h> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        HeaderMut(HeaderRef(ptr), PhantomData)
    }
}

impl PartialEq for Header<'_> {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_ref(), other.as_ref());
        a.field() == b.field() && a.value() == b.value()
    }
}

impl Eq for Header<'_> {}

impl fmt::Debug for Header<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'h> HeaderRef<'a, 'h> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_commit_header *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_commit_header {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The field name, borrowed for `'h`, as it was set.
    ///
    /// Field: git_commit_header.field
    pub fn field(&self) -> &'h CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live header. `new` and
        // every setter store a `&'h CStr`'s pointer, a NUL-terminated string that
        // outlives `'h` and that nothing writes, being shared-borrowed.
        unsafe { CStr::from_ptr(addr_of!((*p).field).read()) }
    }

    /// The value, borrowed for `'h`, as it was set.
    ///
    /// Field: git_commit_header.value
    pub fn value(&self) -> &'h CStr {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to a live header. `new` and
        // every setter store a `&'h CStr`'s pointer, a NUL-terminated string that
        // outlives `'h` and that nothing writes, being shared-borrowed.
        unsafe { CStr::from_ptr(addr_of!((*p).value).read()) }
    }

    /// A copy of the header, as a value of its own, borrowing the same strings.
    pub fn to_header(&self) -> Header<'h> {
        Header::new(self.field(), self.value())
    }
}

impl fmt::Debug for HeaderRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Header")
            .field("field", &self.field())
            .field("value", &self.value())
            .finish()
    }
}

impl<'h> HeaderMut<'_, 'h> {
    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_commit_header {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> HeaderRef<'_, 'h> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> HeaderMut<'_, 'h> {
        HeaderMut(self.0, PhantomData)
    }

    /// Replaces the field name; the old string, borrowed, needs no release.
    ///
    /// Field: git_commit_header.field
    pub fn set_field(&mut self, field: &'h CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live header of a
        // `&'h CStr`'s pointer, which keeps the getters' guarantee; the old pointer is
        // borrowed, so nothing is released.
        unsafe { addr_of_mut!((*p).field).write(field.as_ptr()) }
    }

    /// Replaces the value; the old string, borrowed, needs no release.
    ///
    /// Field: git_commit_header.value
    pub fn set_value(&mut self, value: &'h CStr) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to a live header of a
        // `&'h CStr`'s pointer, which keeps the getters' guarantee; the old pointer is
        // borrowed, so nothing is released.
        unsafe { addr_of_mut!((*p).value).write(value.as_ptr()) }
    }
}

impl fmt::Debug for HeaderMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `Header` is a `git_commit_header`, so a `[Header]` is the array C reads in place.
const _: () = assert!(
    size_of::<Header<'static>>() == size_of::<ffi::git_commit_header>()
        && align_of::<Header<'static>>() == align_of::<ffi::git_commit_header>()
        && size_of::<[Header<'static>; 3]>() == size_of::<[ffi::git_commit_header; 3]>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::sys::errors::runtime_pinned;
    use core::ffi::CStr;
    use core::mem::size_of;

    /// The fixture repositories: `testrepo.git` (a branch at `a65fedf`), `mergedrepo`
    /// (merging `e280915` into `3a34580`) and `empty_bare.git` (an unborn `HEAD`).
    const TESTREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/testrepo.git\0"
    );
    const MERGEDREPO: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/mergedrepo/.gitted\0"
    );
    const EMPTY: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/resources/empty_bare.git\0"
    );

    /// Opens the repository at the NUL-terminated `path`, read-only use only.
    fn open(path: &str) -> *mut ffi::git_repository {
        assert!(runtime_pinned());
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no ceiling
        // directories.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &mut repo,
                path.as_ptr().cast(),
                ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        repo
    }

    /// The hex id of the live commit `commit`.
    fn hex(commit: *mut ffi::git_commit) -> String {
        let mut out = [0u8; 41];
        // SAFETY: `commit` is a live commit the caller's array keeps alive; its id is
        // formatted into `out`, 41 writable bytes, NUL included.
        unsafe {
            ffi::git_oid_tostr(
                out.as_mut_ptr().cast(),
                out.len(),
                ffi::git_commit_id(commit),
            )
        };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn ids(array: CommitArrayRef<'_>) -> Vec<String> {
        array.commit_ptrs().elems().map(hex).collect()
    }

    #[test]
    fn header_layout_matches_c() {
        assert_eq!(size_of::<Header<'_>>(), size_of::<ffi::git_commit_header>());
        assert_eq!(
            size_of::<HeaderRef<'_, '_>>(),
            size_of::<*const ffi::git_commit_header>()
        );
        assert_eq!(
            size_of::<HeaderMut<'_, '_>>(),
            size_of::<*const ffi::git_commit_header>()
        );
    }

    /// `new` stores the strings' own pointers, which the getters read back and C sees
    /// in place, through the header and through the first element of an array.
    #[test]
    fn header_stores_the_borrowed_pointers() {
        let (field, value) = (c"line_one", c"First extra header");
        let header = Header::new(field, value);
        let r = header.as_ref();
        assert_eq!(r.field(), field);
        assert_eq!(r.value(), value);
        assert_eq!(r.field().as_ptr(), field.as_ptr());
        assert_eq!(r.value().as_ptr(), value.as_ptr());
        // SAFETY: the handle's pointer addresses `header`, live and unwritten here.
        let raw = unsafe { r.as_ptr().read() };
        assert_eq!((raw.field, raw.value), (field.as_ptr(), value.as_ptr()));

        let headers = [header, Header::new(c"line_two", c"Second extra header")];
        let array: *const ffi::git_commit_header = headers.as_ptr().cast();
        // SAFETY: `Header` is transparent over `git_commit_header`, so `array` addresses
        // two live C structs, the array's elements, read only here.
        let second = unsafe { array.add(1).read() };
        // SAFETY: the second header stores a `&'static CStr`'s pointer.
        assert_eq!(unsafe { CStr::from_ptr(second.field) }, c"line_two");
    }

    /// The setters replace one string each, through a handle and a reborrow; a copy is
    /// independent of the header it came from.
    #[test]
    fn header_setters_reborrows_and_copies() {
        let owned_field = std::ffi::CString::new("owned").unwrap();
        let mut header = Header::new(c"a", c"b");
        {
            let mut m = header.as_mut();
            m.set_field(&owned_field);
            {
                let mut inner = m.as_mut();
                inner.set_value(c"multi\nline");
                assert_eq!(inner.as_ref().field(), c"owned");
            }
            assert_eq!(m.as_ref().value(), c"multi\nline");
            assert_eq!(
                format!("{m:?}"),
                r#"Header { field: "owned", value: "multi\nline" }"#
            );
        }
        let copy = header.as_ref().to_header();
        let mut second = header;
        second.as_mut().set_value(c"");
        assert_eq!(header.as_ref().value(), c"multi\nline");
        assert_eq!(second.as_ref().value(), c"");
        assert_eq!(copy, header);
        assert_ne!(copy, second);
        assert_eq!(format!("{header:?}"), format!("{:?}", header.as_ref()));
    }

    /// Equality compares the strings, not their addresses.
    #[test]
    fn header_equality_is_by_content() {
        let field = std::ffi::CString::new("line_one").unwrap();
        let value = std::ffi::CString::new("x").unwrap();
        assert_eq!(Header::new(&field, &value), Header::new(c"line_one", c"x"));
        assert_ne!(Header::new(&field, &value), Header::new(c"line_one", c"y"));
        assert_ne!(Header::new(&field, &value), Header::new(c"line_two", c"x"));
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<CommitArray>(), size_of::<ffi::git_commitarray>());
        assert_eq!(
            size_of::<CommitArrayVal>(),
            size_of::<ffi::git_commitarray>()
        );
        assert_eq!(
            size_of::<CommitArrayRef<'_>>(),
            size_of::<*const ffi::git_commitarray>()
        );
        assert_eq!(
            size_of::<CommitArrayMut<'_>>(),
            size_of::<*const ffi::git_commitarray>()
        );
        assert_eq!(size_of::<CommitArrayDispose>(), 0);
    }

    /// The empty array's accessors over a bare value and an owner, without libgit2, so
    /// Miri runs them; nothing here reaches C, `clear` and the drops included.
    #[test]
    fn empty_array_accessors_without_c() {
        let mut array = CommitArray::new();
        let mut m = array.as_mut();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().is_empty());
        assert!(m.as_ref().commit_ptrs().is_empty());
        assert!(m.as_ref().commit_ptrs().elem(0).is_none());
        m.clear();
        assert_eq!(format!("{m:?}"), "[]");
        assert_eq!(format!("{:?}", array.as_ref()), "[]");
        let raw = array.into_inner().into_raw();
        assert!(raw.commits.is_null() && raw.count == 0);
        // SAFETY: a NULL `commits` carries no requirement.
        let adopted = unsafe { CommitArray::from_raw(raw) };
        assert!(adopted.as_ref().is_empty());
    }

    /// Without a buffer the array is empty and owns nothing, whatever `count` says:
    /// every accessor agrees, `clear` resets the count, and neither it nor the dispose
    /// policy calls libgit2 (Miri aborts on any foreign call).
    #[test]
    fn null_buffer_is_empty_and_disposes_without_c() {
        let mut broken = ffi::git_commitarray {
            commits: ptr::null(),
            count: 3,
        };
        // SAFETY: `broken` is live and holds no buffer; nothing is released.
        unsafe { CommitArrayDispose.c_dispose(NonNull::from(&mut broken).cast::<CommitArray>()) };
        assert_eq!((broken.commits, broken.count), (ptr::null(), 3));
        // SAFETY: `broken` is live for the handle's use; nothing else reaches it.
        let mut m = unsafe { CommitArrayMut::from_ptr(&mut broken) }.unwrap();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().commit_ptrs().is_empty());
        assert_eq!(format!("{m:?}"), "[]");
        m.clear();
        assert_eq!((broken.commits, broken.count), (ptr::null(), 0));

        // SAFETY: a NULL `commits` carries no requirement.
        let adopted = unsafe {
            CommitArray::from_raw(ffi::git_commitarray {
                commits: ptr::null(),
                count: usize::MAX,
            })
        };
        assert_eq!(adopted.as_ref().len(), 0);
        drop(adopted);
    }

    /// The handles' projections over a buffer Rust allocated, so Miri checks them: the
    /// pointers are only copied out, never dereferenced, and the buffer leaves through
    /// `into_raw`, so nothing reaches libgit2's dispose.
    #[test]
    fn handles_over_rust_storage() {
        let mut slots = [0u64; 3];
        let fake: Vec<*mut ffi::git_commit> =
            slots.iter_mut().map(|s| ptr::from_mut(s).cast()).collect();
        let raw = ffi::git_commitarray {
            commits: fake.as_ptr(),
            count: 3,
        };
        // SAFETY: never disposed: the value leaves through `into_raw` below, so the
        // test's buffer and fake commits never reach `git_commitarray_dispose`.
        let array = unsafe { CommitArray::from_raw(raw) };
        let r = array.as_ref();
        assert_eq!(r.len(), 3);
        assert!(!r.is_empty());
        let run = r.commit_ptrs();
        assert_eq!(run.len(), 3);
        assert_eq!(run.elems().collect::<Vec<_>>(), fake);
        assert_eq!(run.elem(2), Some(fake[2]));
        assert!(run.elem(3).is_none());
        let tail = run.slice(1..).unwrap();
        assert_eq!(tail.elem(0), Some(fake[1]));
        assert_eq!(format!("{r:?}"), format!("{fake:?}"));
        let back = array.into_inner().into_raw();
        assert_eq!((back.commits, back.count), (fake.as_ptr(), 3));
    }

    /// libgit2 fills an owner's storage in place, as the `git_repository_commit_parents`
    /// wrapper will through the crate-internal seam: one parent for a branch, two for
    /// a merge in progress, none for an unborn `HEAD`. Each result is read through the
    /// handles, cleared before the next fill (libgit2 overwrites without releasing),
    /// and released after its repositories are freed (LSan flags a leak, ASan a double
    /// release or a use after free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_fills_an_owner_in_place() {
        let (testrepo, merged, empty) = (open(TESTREPO), open(MERGEDREPO), open(EMPTY));
        let mut array = CommitArray::new();
        let fill = |array: &mut CommitArrayVal, repo| {
            let out = array.as_mut().as_mut_ptr();
            // SAFETY: `out` is the owner's live storage, reached through its exclusive
            // handle and used for the call alone; it holds the empty array, so nothing
            // is overwritten unreleased; `repo` is open. libgit2 leaves `{NULL, 0}` or a
            // buffer of commit references it owns, which keeps the handles' invariant.
            let rc = unsafe { ffi::git_repository_commit_parents(out, repo) };
            assert_eq!(rc, 0);
        };

        fill(&mut array, testrepo);
        assert_eq!(
            ids(array.as_ref()),
            ["a65fedf39aefe402d3bb6e24df4d4f5fe4547750"]
        );
        array.as_mut().clear();
        assert!(array.as_ref().is_empty());

        fill(&mut array, merged);
        assert_eq!(
            ids(array.as_ref()),
            [
                "3a34580a35add43a4cf361e8e9a30060a905c876",
                "e2809157a7766f272e4cfe26e61ef2678a5357ff"
            ]
        );
        let mut second = CommitArray::new();
        fill(&mut second, testrepo);
        array.as_mut().clear();

        fill(&mut array, empty);
        assert!(array.as_ref().is_empty());
        assert!(array.as_ref().commit_ptrs().is_empty());

        for repo in [testrepo, merged, empty] {
            // SAFETY: each repository is open and freed once; the arrays' commits
            // outlive it, `git_commit_free` reading only the commit.
            unsafe { ffi::git_repository_free(repo) };
        }
        assert_eq!(
            ids(second.as_ref()),
            ["a65fedf39aefe402d3bb6e24df4d4f5fe4547750"]
        );
        drop(second);
        drop(array);
    }

    /// The dispose policy driven by hand, and `into_raw` handing an array back to C's
    /// `git_commitarray_dispose`: each releases every commit and the buffer once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn dispose_policy_and_c_release_the_commits() {
        let repo = open(MERGEDREPO);
        let mut raw = ffi::git_commitarray::default();
        // SAFETY: `raw` is a zeroed array, `repo` open.
        let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
        assert_eq!(rc, 0);
        // SAFETY: `raw` is what libgit2 filled: two commit references in a libgit2
        // buffer, given up here.
        let mut bare = unsafe { CommitArray::from_raw(raw) }.into_inner();
        assert_eq!(bare.as_ref().len(), 2);
        // SAFETY: `bare` is a live array Rust owns, disposed once; C zeroes it.
        unsafe { CommitArrayDispose.c_dispose(NonNull::from(&mut bare)) };
        let zeroed = bare.into_raw();
        assert!(zeroed.commits.is_null() && zeroed.count == 0);

        let mut raw = ffi::git_commitarray::default();
        // SAFETY: as above.
        let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
        assert_eq!(rc, 0);
        // SAFETY: as above.
        let adopted = unsafe { CommitArray::from_raw(raw) };
        let mut back = adopted.into_inner().into_raw();
        assert_eq!(back.count, 2);
        // SAFETY: `back` owns its commits and buffer again, released once by C.
        unsafe { ffi::git_commitarray_dispose(&mut back) };
        // SAFETY: `repo` is open and freed once.
        unsafe { ffi::git_repository_free(repo) };
    }
}
