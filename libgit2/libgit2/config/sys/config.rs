//! Safe wrappers over `include/git2/sys/config.h`.
//!
//! Memory backend options are caller-owned values. Their nullable strings are
//! borrowed, never freed by C. There is no C allocation, destruction or cloning
//! primitive for the options themselves. A Rust clone simply shares the immutable string borrows,
//! and the options cross threads as the borrows they hold do.
//!
//! Backend entries retain immutable storage via an allocation-specific callback.
//! Acquired references use [`BackendEntryArc`]; borrowed handles never release
//! storage. [`BackendEntry::from_entry`] safely exposes the full backend layout
//! of the public entry owner's already-required enclosing allocation.

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use std::ffi::CString;

use ffibox::{CArc, CBorrowedPtr, CCell, CDrop, define_ctype};

use crate::libgit2::config::config::{
    ConfigError, ConfigLevel, ConfigMut, EntryArc, EntryMut, EntryRef,
};
use crate::libgit2::util::errors::ErrorClass;
use crate::libgit2::util::sys::errors::set;

use crate::ffi;

/// Caller-owned memory backend options, borrowing strings for `'o`.
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendMemoryOptions;
/// use std::ffi::CString;
/// let mut options = BackendMemoryOptions::new();
/// {
///     let label = CString::new("temporary").unwrap();
///     options.as_mut().set_backend_type(Some(&label));
/// }
/// assert!(options.as_ref().backend_type().is_some());
/// ```
///
/// A mutable handle cannot shrink the stored-borrow lifetime:
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendMemoryOptionsMut;
/// fn shrink<'a, 'o>(m: BackendMemoryOptionsMut<'a, 'static>)
///     -> BackendMemoryOptionsMut<'a, 'o> { m }
/// ```
///
/// Neither a handle nor a string view can outlive the options:
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendMemoryOptions;
/// let mut options = BackendMemoryOptions::new();
/// options.as_mut().set_backend_type(Some(c"label"));
/// let view = options.as_ref().backend_type();
/// drop(options);
/// assert!(view.is_some());
/// ```
///
/// Shared string views prevent a concurrent mutable reborrow:
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendMemoryOptions;
/// let mut options = BackendMemoryOptions::new();
/// options.as_mut().set_origin_path(Some(c"origin"));
/// let shared = options.as_ref();
/// let path = shared.origin_path();
/// options.as_mut().set_origin_path(None);
/// assert!(path.is_some());
/// ```
///
/// The options cross threads, but only with borrows that outlive the thread:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::config::sys::config::BackendMemoryOptions;
/// use std::ffi::CString;
/// let label = CString::new("temporary").unwrap();
/// let mut options = BackendMemoryOptions::new();
/// options.as_mut().set_backend_type(Some(&label));
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Wraps: git_config_backend_memory_options
/// Handwritten
#[repr(transparent)]
pub struct BackendMemoryOptions<'o>(
    ffi::git_config_backend_memory_options,
    PhantomData<(*const (), &'o CStr)>,
);

/// Copyable shared view of memory backend options.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct BackendMemoryOptionsRef<'a, 'o>(CBorrowedPtr<'a, BackendMemoryOptions<'o>>);

/// Exclusive view, invariant in the stored-borrow lifetime.
///
/// Handwritten
#[repr(transparent)]
pub struct BackendMemoryOptionsMut<'a, 'o>(
    BackendMemoryOptionsRef<'a, 'o>,
    PhantomData<&'a mut BackendMemoryOptions<'o>>,
);

// SAFETY: layout and handles are transparent over the raw struct and CBorrowedPtr,
// respectively, with only zero-sized markers and no handle Drop. No handle forms a
// reference to C memory. Mut is invariant in Self through its exclusive marker.
unsafe impl<'o> CCell for BackendMemoryOptions<'o> {
    type C = ffi::git_config_backend_memory_options;
    type Ref<'a>
        = BackendMemoryOptionsRef<'a, 'o>
    where
        Self: 'a;
    type Mut<'a>
        = BackendMemoryOptionsMut<'a, 'o>
    where
        Self: 'a;
}

// SAFETY: the options are a version and two pointers that only the setters write, each
// NULL or a `&'o CStr`'s: a shared, immutable borrow of `CStr`, which is `Sync`, so it
// may be read from any thread. Nothing has thread affinity or interior mutability, and
// nothing is released on drop, so moving the options moves bytes.
unsafe impl Send for BackendMemoryOptions<'_> {}
// SAFETY: as above; `BackendMemoryOptionsMut` is an exclusive borrow, as `&mut`.
unsafe impl Send for BackendMemoryOptionsMut<'_, '_> {}

// SAFETY: as above, and shared access only reads the fields (every write goes through
// `BackendMemoryOptionsMut`, which excludes other handles), so readers race nothing.
unsafe impl Sync for BackendMemoryOptions<'_> {}
// SAFETY: as above; `BackendMemoryOptionsRef` is a shared borrow, as `&`.
unsafe impl Send for BackendMemoryOptionsRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for BackendMemoryOptionsRef<'_, '_> {}
// SAFETY: `&BackendMemoryOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for BackendMemoryOptionsMut<'_, '_> {}

impl<'o> BackendMemoryOptions<'o> {
    /// C's defaults: current version and both strings unset. No library
    /// initialization is needed: this version's init only copies a template.
    ///
    /// Wraps: git_config_backend_memory_options_init
    pub fn new() -> Self {
        let mut raw = MaybeUninit::uninit();
        // SAFETY: raw provides exclusive writable storage; init fills the entire
        // struct for its supported version, without retaining any pointer.
        let rc = unsafe {
            ffi::git_config_backend_memory_options_init(
                raw.as_mut_ptr(),
                ffi::GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0, "memory options init refused its own version");
        // SAFETY: success initialized every field from the C template.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    pub fn as_ref(&self) -> BackendMemoryOptionsRef<'_, 'o> {
        // SAFETY: initialized Rust-owned storage, shared for the handle's lifetime.
        BackendMemoryOptionsRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    pub fn as_mut(&mut self) -> BackendMemoryOptionsMut<'_, 'o> {
        // SAFETY: exclusive borrow of initialized storage supplies write provenance;
        // no other handle can be used while this result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        BackendMemoryOptionsMut(BackendMemoryOptionsRef(ptr), PhantomData)
    }
}

impl Default for BackendMemoryOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for BackendMemoryOptions<'_> {
    fn clone(&self) -> Self {
        // Both strings are immutable borrows, not resources owned by the struct.
        Self(self.0, PhantomData)
    }
}

impl<'a, 'o> BackendMemoryOptionsRef<'a, 'o> {
    /// # Safety
    /// `ptr` is NULL or aligned initialized options, readable and unchanged for
    /// `'a`; both strings are NULL or NUL-terminated immutable borrows for `'o`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(
        ptr: *const ffi::git_config_backend_memory_options,
    ) -> Option<Self> {
        let ptr = NonNull::new(ptr.cast_mut().cast::<BackendMemoryOptions<'o>>())?;
        // SAFETY: caller proves liveness, initialized fields and referent lifetimes;
        // the layout cast is transparent.
        Some(Self(unsafe { CBorrowedPtr::new(ptr) }))
    }

    pub(crate) fn as_ptr(&self) -> *const ffi::git_config_backend_memory_options {
        self.0.as_non_null().as_ptr().cast()
    }

    /// Structure version; constructors use the current supported version.
    ///
    /// Field: git_config_backend_memory_options.version
    pub fn version(&self) -> u32 {
        // SAFETY: reads the initialized scalar through the live shared handle.
        unsafe { addr_of!((*self.as_ptr()).version).read() }
    }

    /// Unset means C will use "in-memory"; an empty string is distinct from unset.
    ///
    /// Field: git_config_backend_memory_options.backend_type
    pub fn backend_type(&self) -> Option<&'a CStr> {
        // SAFETY: reads the initialized pointer through the live shared handle.
        let value = unsafe { addr_of!((*self.as_ptr()).backend_type).read() };
        if value.is_null() {
            None
        } else {
            // SAFETY: handle invariant ensures a NUL-terminated immutable borrow
            // for 'o, which outlives this handle's 'a.
            Some(unsafe { CStr::from_ptr(value) })
        }
    }

    /// Unset means entries have no origin; empty and non-UTF-8 strings are valid.
    ///
    /// Field: git_config_backend_memory_options.origin_path
    pub fn origin_path(&self) -> Option<&'a CStr> {
        // SAFETY: reads the initialized pointer through the live shared handle.
        let value = unsafe { addr_of!((*self.as_ptr()).origin_path).read() };
        if value.is_null() {
            None
        } else {
            // SAFETY: as backend_type, referent is immutable and valid for 'a.
            Some(unsafe { CStr::from_ptr(value) })
        }
    }
}

impl fmt::Debug for BackendMemoryOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for BackendMemoryOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackendMemoryOptions")
            .field("version", &self.version())
            .field("backend_type", &self.backend_type())
            .field("origin_path", &self.origin_path())
            .finish()
    }
}

impl fmt::Debug for BackendMemoryOptionsMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'o> BackendMemoryOptionsMut<'a, 'o> {
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_config_backend_memory_options {
        self.0.0.as_non_null().as_ptr().cast()
    }

    pub fn as_ref(&self) -> BackendMemoryOptionsRef<'_, 'o> {
        self.0
    }

    pub fn as_mut(&mut self) -> BackendMemoryOptionsMut<'_, 'o> {
        BackendMemoryOptionsMut(self.0, PhantomData)
    }

    /// Sets the scalar version. C's memory backend constructors currently ignore
    /// it; normally leave this at the constructor's supported version.
    pub fn set_version(&mut self, version: u32) {
        // SAFETY: exclusive live handle permits writing this initialized scalar.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).version).write(version) };
    }

    /// Replaces the stored immutable borrow; never frees either string.
    pub fn set_backend_type(&mut self, value: Option<&'o CStr>) {
        let value = value.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: exclusive live handle permits replacing the pointer; the new
        // referent is NUL-terminated and borrowed for the options' 'o.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).backend_type).write(value) };
    }

    /// Replaces the stored immutable borrow; never frees either string.
    pub fn set_origin_path(&mut self, value: Option<&'o CStr>) {
        let value = value.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: as set_backend_type, exclusive field access and borrow for 'o.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).origin_path).write(value) };
    }
}

define_ctype!(
    /// A backend entry, including its storage-specific release callback.
    ///
    /// Lookups acquire a reference to immutable backing storage, whereas an
    /// iterator only lends an entry. Even acquired references may alias: use
    /// [`BackendEntryArc`], never a sole-owner box for a backend lookup result.
    /// There is no public C up-ref or standalone copy primitive. The callback
    /// may be thread-bound, so no thread-safety traits are asserted.
    ///
    /// The callback is fixed by the allocator, not a replaceable user hook;
    /// replacing it could release unrelated storage. The embedded entry's
    /// strings are immutable borrows, not independently owned fields.
    ///
    /// An acquired owner has no exclusive handle, neither proven sole nor
    /// taken back as a box:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::config::sys::config::BackendEntryArc;
    /// fn mutate(mut entry: BackendEntryArc) { let _ = entry.get_mut(); }
    /// ```
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::config::sys::config::BackendEntryArc;
    /// fn unshare(entry: BackendEntryArc) { let _ = entry.try_into_box(); }
    /// ```
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::config::sys::config::BackendEntryArc;
    /// fn escape(entry: BackendEntryArc) {
    ///     let name = entry.as_ref().entry().name();
    ///     drop(entry);
    ///     assert!(name.is_some());
    /// }
    /// ```
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::config::sys::config::BackendEntryArc;
    /// fn clone(entry: BackendEntryArc) { let _ = entry.clone(); }
    /// ```
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::config::sys::config::BackendEntryArc;
    /// fn send(entry: BackendEntryArc) { std::thread::spawn(move || drop(entry)); }
    /// ```
    ///
    /// Wraps: git_config_backend_entry
    BackendEntry, BackendEntryRef, BackendEntryMut, ffi::git_config_backend_entry
);

/// Releases one acquired reference through the entry's own callback. State is
/// recovered from the object; no callback from a different allocation is saved.
/// The same C routine as [`EntryFree`](crate::libgit2::config::config::EntryFree),
/// given the enclosing layout.
///
/// Wraps: git_config_entry_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct BackendEntryFree;

// SAFETY: an owned backend entry holds one acquired reference and a matching release
// callback. `git_config_entry_free` takes the embedded public entry, casts it back to
// this enclosing layout and lets the callback consume exactly that reference, without
// assuming the allocation is only as large as the public entry.
unsafe impl CDrop<BackendEntry> for BackendEntryFree {
    unsafe fn c_drop(&self, ptr: NonNull<BackendEntry>) {
        let entry = ptr.as_ptr().cast::<ffi::git_config_backend_entry>();
        // SAFETY: per `c_drop`'s contract, the caller gives up a live backend entry's
        // reference; its public entry is the offset-zero inline field, so C's cast
        // recovers this allocation.
        unsafe { ffi::git_config_entry_free(addr_of_mut!((*entry).entry)) }
    }
}

/// One acquired reference; intentionally neither cloneable nor mutable.
pub type BackendEntryArc = CArc<BackendEntry, BackendEntryFree>;

impl BackendEntry {
    /// Promotes an acquired public entry to its full backend layout, without
    /// copying or acquiring another reference. `EntryArc`'s release contract
    /// already requires this larger allocation, unlike an arbitrary `EntryRef`.
    pub fn from_entry(entry: EntryArc) -> BackendEntryArc {
        let ptr = entry.into_c();
        // SAFETY: EntryArc owns an acquired backend entry reference (C's release
        // routine casts it to this exact enclosing type). entry is at offset
        // zero; transferring the owner preserves its callback and keepalive.
        unsafe { BackendEntryArc::from_c(ptr.cast()) }.expect("entry owner is non-null")
    }

    /// Returns the public-entry owner, transferring the same single reference.
    pub fn into_entry(entry: BackendEntryArc) -> EntryArc {
        let ptr = entry.into_c();
        // SAFETY: BackendEntryArc owns an acquired reference, and its first field
        // is the public entry. EntryFree invokes the same enclosing callback.
        unsafe { EntryArc::from_c(ptr.cast()) }.expect("backend entry owner is non-null")
    }
}

impl<'a> BackendEntryRef<'a> {
    /// Shared view of the inline entry, kept alive by this backend-entry borrow.
    ///
    /// Field: git_config_backend_entry.entry
    pub fn entry(&self) -> EntryRef<'a> {
        // SAFETY: projects the initialized inline field, without forming a Rust
        // reference to foreign storage. Its immutable strings live for 'a.
        unsafe { EntryRef::from_ptr(addr_of!((*self.as_ptr()).entry).cast_mut()) }
            .expect("inline entry is non-null")
    }

    /// The allocation-matched release strategy, not an unbound function pointer.
    /// Calling it requires moving an acquired owner; a borrowed iterator entry
    /// cannot be freed. The strategy recovers the callback from the owner it is
    /// given, never from a different entry that supplied this token.
    ///
    /// Field: git_config_backend_entry.free
    pub fn release_policy(&self) -> BackendEntryFree {
        // SAFETY: reads the initialized callback through the live shared handle.
        let callback = unsafe { addr_of!((*self.as_ptr()).free).read() };
        assert!(
            callback.is_some(),
            "backend entry lacks its release callback"
        );
        BackendEntryFree
    }
}

impl BackendEntryMut<'_> {
    /// Exclusive view of the inline entry. Only genuinely exclusive storage
    /// (not an acquired backend lookup reference) can supply this handle.
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::config::sys::config::BackendEntryMut;
    /// fn alias(mut parent: BackendEntryMut<'_>) {
    ///     let mut child = parent.entry_mut();
    ///     let other = parent.as_mut();
    ///     assert!(child.as_ref().name().is_some());
    ///     drop(other);
    /// }
    /// ```
    pub fn entry_mut(&mut self) -> EntryMut<'_> {
        // SAFETY: the exclusive parent borrow includes its inline field; the
        // child cannot outlive this reborrow or alias another parent handle.
        unsafe { EntryMut::from_ptr(addr_of_mut!((*self.as_mut_ptr()).entry)) }
            .expect("inline entry is non-null")
    }
}

impl BackendEntryFree {
    /// Invokes an owner's release callback exactly once, consuming its reference.
    /// This is also what dropping the owner does; borrowed handles cannot call it.
    ///
    /// ```compile_fail,E0308
    /// use libgit2::libgit2::config::sys::config::{BackendEntryFree, BackendEntryRef};
    /// fn free_borrowed(entry: BackendEntryRef<'_>) { BackendEntryFree.release(entry); }
    /// ```
    pub fn release(self, entry: BackendEntryArc) {
        drop(entry);
    }
}

impl fmt::Debug for BackendEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// The callback's address says nothing portable, so only the entry is printed.
impl fmt::Debug for BackendEntryRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackendEntry")
            .field("entry", &self.entry())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for BackendEntryMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// A C iterator whose enclosing allocation and callbacks stay paired.
///
/// The iterator borrows its config for `'c`: C's all-iterator stores a raw
/// config pointer without retaining it. The layout is never copied, cloned or
/// zero-constructed. Each backend iterator can be freed on an advance, so
/// entries borrow the exclusive advance, not the whole iterator's lifetime.
/// Neither owners nor handles cross threads. Callback slots are allocation
/// identity, not replaceable user hooks.
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::config::config::Config;
/// let config = Config::new().unwrap();
/// let iter = config.as_ref().iterator().unwrap();
/// drop(config);
/// drop(iter);
/// ```
///
/// ```compile_fail,E0499
/// use libgit2::libgit2::config::config::Config;
/// let config = Config::new().unwrap();
/// let mut iter = config.as_ref().iterator().unwrap();
/// let mut m = iter.as_mut();
/// let entry = m.advance().unwrap();
/// let _ = m.advance();
/// let _ = entry.unwrap().entry().name();
/// ```
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::config::config::Config;
/// let config = Config::new().unwrap();
/// let mut iter = config.as_ref().iterator().unwrap();
/// let mut m = iter.as_mut();
/// let entry = m.advance().unwrap();
/// drop(iter);
/// let _ = entry.unwrap().entry().name();
/// ```
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::config::sys::config::IteratorBox;
/// fn send(iter: IteratorBox<'static>) { std::thread::spawn(move || drop(iter)); }
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::IteratorMut;
/// fn shrink<'a, 'c>(m: IteratorMut<'a, 'static>) -> IteratorMut<'a, 'c> { m }
/// ```
///
/// An iterator also freezes mutation of the borrowed config:
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::config::config::Config;
/// let mut config = Config::new().unwrap();
/// let iter = config.as_ref().iterator().unwrap();
/// config.as_mut().set_string(c"a.b", c"changed").unwrap();
/// drop(iter);
/// ```
///
/// ```compile_fail,E0308
/// use libgit2::libgit2::config::sys::config::{IteratorFree, IteratorRef};
/// fn free_borrowed(iter: IteratorRef<'_, '_>) { IteratorFree.release(iter); }
/// ```
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::config::sys::config::IteratorBox;
/// fn copy(iter: IteratorBox<'_>) { let _ = iter.clone(); }
/// ```
///
/// Wraps: git_config_iterator
/// Handwritten
#[repr(transparent)]
pub struct Iterator<'c>(ffi::git_config_iterator, PhantomData<(*const (), &'c ())>);

/// Copyable shared view. It cannot advance or free the allocation.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct IteratorRef<'a, 'c>(CBorrowedPtr<'a, Iterator<'c>>);

/// Exclusive iterator view, invariant in its config keepalive.
///
/// Handwritten
#[repr(transparent)]
pub struct IteratorMut<'a, 'c>(IteratorRef<'a, 'c>, PhantomData<&'a mut Iterator<'c>>);

// SAFETY: the layout is transparent over the initialized C base; handles are
// transparent over CBorrowedPtr with no Drop. The mutable handle is invariant
// in Iterator<'c>, and shared handles provide only reads, never advancement.
unsafe impl<'c> CCell for Iterator<'c> {
    type C = ffi::git_config_iterator;
    type Ref<'a>
        = IteratorRef<'a, 'c>
    where
        Self: 'a;
    type Mut<'a>
        = IteratorMut<'a, 'c>
    where
        Self: 'a;
}

/// Stateless release dispatch, recovered from the allocation being released.
///
/// Wraps: git_config_iterator_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct IteratorFree;

// SAFETY: adopted owners hold a complete, uniquely owned enclosing iterator
// allocation and its matching free callback. C dispatches that callback once,
// releasing nested iterators, entry lists, compiled regexes and storage.
unsafe impl<'c> CDrop<Iterator<'c>> for IteratorFree {
    unsafe fn c_drop(&self, p: NonNull<Iterator<'c>>) {
        // SAFETY: the caller transfers one live complete iterator allocation;
        // its callback matches its allocator and all dependencies remain live.
        unsafe { ffi::git_config_iterator_free(p.as_ptr().cast()) }
    }
}

/// Sole owner of the full C iterator allocation (not just its public base).
pub type IteratorBox<'c> = ffibox::CBox<Iterator<'c>, IteratorFree>;

impl<'c> Iterator<'c> {
    pub fn as_ref(&self) -> IteratorRef<'_, 'c> {
        // SAFETY: initialized inline Rust-owned layout, borrowed for this call;
        // raw projection avoids forming a reference to any C-owned pointee.
        unsafe { IteratorRef::from_ptr(addr_of!(self.0).cast_mut()) }.unwrap()
    }

    pub fn as_mut(&mut self) -> IteratorMut<'_, 'c> {
        let p = NonNull::from(self);
        // SAFETY: exclusively borrowed initialized inline Rust-owned layout.
        IteratorMut(IteratorRef(unsafe { CBorrowedPtr::new(p) }), PhantomData)
    }
}

impl fmt::Debug for Iterator<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl IteratorFree {
    /// Consumes an owner, never a borrowed entry or iterator handle.
    pub fn release(self, iterator: IteratorBox<'_>) {
        drop(iterator);
    }
}

impl<'a, 'c> IteratorRef<'a, 'c> {
    pub(crate) fn as_ptr(&self) -> *mut ffi::git_config_iterator {
        self.0.as_non_null().as_ptr().cast()
    }

    /// # Safety
    /// `p` is NULL or a complete initialized iterator with matching callbacks,
    /// live and unmodified for `'a`; its config/backend dependencies outlive `'c`.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(p: *mut ffi::git_config_iterator) -> Option<Self> {
        // SAFETY: the contract supplies initialized live storage for 'a.
        NonNull::new(p.cast()).map(|p| Self(unsafe { CBorrowedPtr::new(p) }))
    }

    /// User/backend flags; libgit2's built-in iterators leave these zero.
    ///
    /// Field: git_config_iterator.flags
    pub fn flags(&self) -> u32 {
        // SAFETY: initialized scalar projected through a live shared handle.
        unsafe { addr_of!((*self.as_ptr()).flags).read() }
    }

    /// Nullable borrowed backend, never an owner. Built-in iterators leave it
    /// NULL. The handle is tied to the iterator borrow and never owns the backend.
    ///
    /// Field: git_config_iterator.backend
    pub fn backend(&self) -> Option<BackendRef<'a>> {
        // SAFETY: the iterator contract keeps its stored backend alive and unchanged
        // for its dependency lifetime, which outlives this shared handle.
        unsafe { BackendRef::from_ptr(addr_of!((*self.as_ptr()).backend).read()) }
    }

    /// Allocation-matched dispatch, not a detachable C function pointer. The
    /// callable always reads the callback of the mutable iterator it receives.
    ///
    /// Field: git_config_iterator.next
    pub fn next_callback(&self) -> IteratorNext {
        IteratorNext
    }

    /// Stateless policy that recovers an owned iterator's own free callback.
    ///
    /// Field: git_config_iterator.free
    pub fn release_policy(&self) -> IteratorFree {
        IteratorFree
    }
}

impl<'a, 'c> IteratorMut<'a, 'c> {
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_config_iterator {
        self.0.as_ptr()
    }

    pub fn as_ref(&self) -> IteratorRef<'_, 'c> {
        self.0
    }

    pub fn as_mut(&mut self) -> IteratorMut<'_, 'c> {
        IteratorMut(self.0, PhantomData)
    }

    pub fn set_flags(&mut self, flags: u32) {
        // SAFETY: the exclusive handle grants writing this independent scalar.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).flags).write(flags) }
    }

    /// Replaces a stored backend borrow without freeing the old one. The lifetime
    /// is typed; allocation-specific callback compatibility remains a caller duty.
    ///
    /// # Safety
    /// The enclosing iterator's next/free callbacks must accept the supplied
    /// backend (including None) and leave its metadata unchanged through every
    /// call; they may not free it or mutate it incompatibly with the shared
    /// borrow. A custom iterator may require its original backend. The typed
    /// borrow already guarantees liveness through the dependency lifetime.
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::Config;
    /// use libgit2::libgit2::config::sys::config::Backend;
    /// let config = Config::new().unwrap();
    /// let mut iter = config.as_ref().iterator().unwrap();
    /// {
    ///     let backend = Backend::new();
    ///     // SAFETY: built-in next/free ignore the slot, but the dead borrow is rejected.
    ///     unsafe { iter.as_mut().set_backend(Some(backend.as_ref())) };
    /// }
    /// drop(iter);
    /// ```
    pub unsafe fn set_backend(&mut self, backend: Option<BackendRef<'c>>) {
        let backend = backend.map_or(ptr::null_mut(), |b| b.as_ptr().cast_mut());
        // SAFETY: exclusive access to the slot; the caller guarantees keepalive
        // and compatibility with the enclosing allocation's callbacks.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).backend).write(backend) }
    }

    /// Advances the allocation's inline callback. `None` is C's ITEROVER; other
    /// errors preserve the code and copied thread-local error. A subsequent call
    /// after an error is allowed (C can proceed to a different backend).
    pub fn advance(&mut self) -> Result<Option<BackendEntryRef<'_>>, ConfigError> {
        IteratorNext.call(self)
    }
}

/// Ownership-compatible callable for the inline `next` slot.
#[derive(Clone, Copy, Debug, Default)]
pub struct IteratorNext;

impl IteratorNext {
    /// The result is borrowed until this mutable reborrow ends: it cannot be
    /// kept across another call, released, or promoted to an acquired owner.
    pub fn call<'a>(
        &self,
        iterator: &'a mut IteratorMut<'_, '_>,
    ) -> Result<Option<BackendEntryRef<'a>>, ConfigError> {
        let p = iterator.as_mut_ptr();
        // SAFETY: reads the initialized callback from the complete live iterator.
        let next = unsafe { addr_of!((*p).next).read() }.expect("iterator lacks next callback");
        let mut entry = ptr::null_mut();
        // SAFETY: exclusive live iterator with its allocation-matched callback;
        // writable output slot, and no previously lent entry is still borrowed.
        let rc = unsafe { next(&mut entry, p) };
        if rc == ffi::GIT_ITEROVER {
            return Ok(None);
        }
        if rc != 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        Ok(Some(
            // SAFETY: callback success lends an initialized immutable entry through
            // the iterator until its next advance/destruction, excluded for 'a.
            unsafe { BackendEntryRef::from_ptr(entry) }
                .expect("successful iterator callback returned NULL"),
        ))
    }
}

// A narrow workaround for C's valueless multivar regex UB. The campaign builds
// libgit2 with USE_REGEX=pcre2; these bindings link the same system PCRE2 library,
// use the same options and preserve arbitrary pattern/value bytes (not UTF-8).
pub(crate) mod value_filter {
    use super::*;
    use crate::libgit2::util::sys::errors::set_oom;
    use ffibox::{CBox, impl_cdrop};
    use pcre2_sys as pcre;

    define_ctype!(Regex, RegexRef, RegexMut, pcre::pcre2_code_8);
    /// PCRE2's compiled-pattern releaser, recovered through ffibox.
    ///
    /// Wraps: pcre2_code_free_8
    #[derive(Clone, Copy, Debug, Default)]
    pub struct RegexFree;
    impl_cdrop!(RegexFree, Regex, pcre::pcre2_code_free_8);
    type RegexBox = CBox<Regex, RegexFree>;

    define_ctype!(
        MatchData,
        MatchDataRef,
        MatchDataMut,
        pcre::pcre2_match_data_8
    );
    /// PCRE2's transient match-data releaser.
    ///
    /// Wraps: pcre2_match_data_free_8
    #[derive(Clone, Copy, Debug, Default)]
    pub struct MatchDataFree;
    impl_cdrop!(MatchDataFree, MatchData, pcre::pcre2_match_data_free_8);
    type MatchDataBox = CBox<MatchData, MatchDataFree>;

    fn compile(pattern: &CStr) -> Result<RegexBox, ConfigError> {
        let mut error = 0;
        let mut offset = 0;
        // SAFETY: readable bytes for the supplied length, writable error slots;
        // default context and options match libgit2's git_regexp_compile. PCRE2
        // copies its pattern and returns its own uniquely owned compiled code.
        let p = unsafe {
            pcre::pcre2_compile_8(
                pattern.as_ptr().cast(),
                pattern.to_bytes().len(),
                0,
                &mut error,
                &mut offset,
                ptr::null_mut(),
            )
        };
        // SAFETY: non-NULL is one compiled-code allocation paired with code_free.
        if let Some(regex) = unsafe { RegexBox::from_c(p) } {
            return Ok(regex);
        }
        let mut message = [0u8; 1024];
        // SAFETY: full writable buffer; PCRE2 writes a NUL-terminated error
        // message when it fits. The initialized zero tail covers failure too.
        unsafe { pcre::pcre2_get_error_message_8(error, message.as_mut_ptr(), message.len()) };
        let message = CStr::from_bytes_until_nul(&message).unwrap_or(c"regex compilation failed");
        set(ErrorClass::Regex, message);
        Err(ConfigError::from_libgit2(ffi::GIT_EINVALIDSPEC))
    }

    fn matches(regex: &RegexBox, value: &CStr) -> bool {
        // SAFETY: default context, one output pair, a fresh match-data allocation.
        let p = unsafe { pcre::pcre2_match_data_create_8(1, ptr::null_mut()) };
        // SAFETY: non-NULL is uniquely owned match data paired with data_free.
        let Some(data) = (unsafe { MatchDataBox::from_c(p) }) else {
            set_oom();
            return false; // C also skips all nonzero match statuses.
        };
        // SAFETY: compiled regex and exclusive match data are live; value is
        // readable for its length (including zero); PCRE2 retains neither borrow.
        let rc = unsafe {
            pcre::pcre2_match_8(
                regex.as_c_ptr(),
                value.as_ptr().cast(),
                value.to_bytes().len(),
                0,
                0,
                data.as_c_ptr(),
                ptr::null_mut(),
            )
        };
        rc >= 0
    }

    /// Rust-owned callback payload: C only accesses its offset-zero ABI base.
    ///
    /// Handwritten
    #[repr(C)]
    struct Filtered<'c> {
        base: Iterator<'c>,
        state: State<'c>,
    }

    struct State<'c> {
        inner: IteratorBox<'c>,
        regex: RegexBox,
    }

    impl Filtered<'_> {
        unsafe extern "C" fn next(
            out: *mut *mut ffi::git_config_backend_entry,
            iterator: *mut ffi::git_config_iterator,
        ) -> i32 {
            // SAFETY: this callback belongs only to Filtered allocations; the caller
            // gives exclusive access and C never retains a pointer to Rust's state.
            // The reference covers Rust-owned state, not the C base or its pointees.
            let state = unsafe { &mut *addr_of_mut!((*iterator.cast::<Self>()).state) };
            loop {
                let mut m = state.inner.as_mut();
                let entry = match m.advance() {
                    Ok(Some(entry)) => entry,
                    Ok(None) => return ffi::GIT_ITEROVER,
                    Err(error) => return error.raw_code(),
                };
                let Some(value) = entry.entry().value() else {
                    continue;
                };
                if !matches(&state.regex, value) {
                    continue;
                }
                // SAFETY: out is a writable slot; this borrowed entry stays pinned
                // by the inner iterator until the next outer advance or release.
                unsafe { out.write(entry.as_ptr().cast_mut()) };
                return 0;
            }
        }

        unsafe extern "C" fn free(iterator: *mut ffi::git_config_iterator) {
            // SAFETY: the callback consumes one uniquely owned Filtered Box whose
            // base is at offset zero. Erased lifetime is used only for destruction;
            // the owner kept the original config live. No lent entry survives here.
            drop(unsafe { Box::from_raw(iterator.cast::<Self>()) });
        }
    }

    pub(crate) fn new<'c>(
        inner: IteratorBox<'c>,
        pattern: &CStr,
    ) -> Result<IteratorBox<'c>, ConfigError> {
        let regex = compile(pattern)?;
        let base = ffi::git_config_iterator {
            backend: ptr::null_mut(),
            flags: 0,
            next: Some(Filtered::next),
            free: Some(Filtered::free),
        };
        let value = Box::new(Filtered {
            base: Iterator(base, PhantomData),
            state: State { inner, regex },
        });
        // SAFETY: complete offset-zero ABI iterator, callbacks matched to this
        // Box. Its payload uniquely owns inner and regex; inner preserves 'c.
        Ok(unsafe { IteratorBox::from_c(Box::into_raw(value).cast()) }.unwrap())
    }
}

impl fmt::Debug for IteratorRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Iterator")
            .field("flags", &self.flags())
            .field("backend", &self.backend())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for IteratorMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

define_ctype!(
    /// Layout-compatible backend base. Callbacks belong to the complete enclosing
    /// allocation: they cannot be replaced or detached from that allocation.
    /// Shared handles only inspect metadata; all dispatch requires exclusivity.
    /// No Send/Sync or Clone is asserted for arbitrary third-party backends.
    ///
    /// Wraps: git_config_backend
    Backend, BackendRef, BackendMut, ffi::git_config_backend
);

/// Release dispatch selected by the allocation, not by another backend's vtable.
///
/// Wraps: git_config_backend_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct BackendFree;

// SAFETY: owners adopt complete backend allocations with their matching free
// callback; dispatch consumes exactly one allocation, including its owned fields.
unsafe impl CDrop<Backend> for BackendFree {
    unsafe fn c_drop(&self, p: NonNull<Backend>) {
        let p = p.as_ptr().cast::<ffi::git_config_backend>();
        // SAFETY: the owned complete allocation has an initialized matching callback.
        let free = unsafe { addr_of!((*p).free).read() }.expect("backend lacks free callback");
        // SAFETY: caller surrenders the sole owner and all borrowed dependencies live.
        unsafe { free(p) };
    }
}

/// Sole owner of an opened backend. A snapshot is a distinct read-only owner,
/// not a bytewise clone or an additional reference to this allocation.
///
/// The ffibox adoption seam requires a complete, exclusively owned, already
/// opened backend with allocation-matched callbacks obeying this public C
/// interface: get acquires an immutable independently retained entry; iterator
/// owns its result; snapshot yields independently owned storage after open;
/// input strings are not retained; snapshots set readonly nonzero. NULL slots
/// other than free are permitted: calls through them return a Config-class
/// error naming the slot.
/// Repository/other external dependencies must remain live through destruction;
/// use a lifetime-carrying owner instead for storage that retains such borrows.
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendBox;
/// fn send(backend: BackendBox) { std::thread::spawn(move || drop(backend)); }
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendBox;
/// fn duplicate(backend: BackendBox) { let _ = backend.clone(); }
/// ```
pub type BackendBox = ffibox::CBox<Backend, BackendFree>;

/// Construction phase: only metadata and a consuming one-time open are available.
/// C retains stable storage throughout initialization; failure frees it normally.
///
/// Handwritten
pub struct UnopenedBackend(BackendBox);

/// Non-dereferenceable identity of C's registration back-pointer. It may be stale:
/// open_level shares a backend and does not keep the original registering config
/// alive. No C consumer reads this field. It is not an owned or live ConfigRef.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendConfigIdentity(NonNull<ffi::git_config>);

impl Backend {
    /// Inert implementation template, not an opened enclosing backend allocation.
    /// Missing callbacks produce errors, never calls through NULL. No resource is
    /// owned by this inline value and it must not be adopted as a BackendBox.
    ///
    /// Wraps: git_config_init_backend
    pub fn new() -> Self {
        let mut raw = MaybeUninit::uninit();
        // SAFETY: supported version initializes the entire exclusive output slot;
        // initialization only copies a template and retains no pointers.
        let rc = unsafe {
            ffi::git_config_init_backend(raw.as_mut_ptr(), ffi::GIT_CONFIG_BACKEND_VERSION)
        };
        assert_eq!(rc, 0, "backend init refused its own version");
        // SAFETY: successful init wrote every field, including nullable callbacks.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::new()
    }
}

/// Refuses a call through an absent callback slot, where C would call through
/// NULL. The error names the slot instead of reporting the thread's last error,
/// which belongs to an unrelated earlier call.
fn missing_slot(slot: &str) -> ConfigError {
    let message = format!("config backend does not implement '{slot}'");
    set(
        ErrorClass::Config,
        &CString::new(message).expect("slot names contain no NUL"),
    );
    ConfigError::from_libgit2(ffi::GIT_ERROR)
}
fn backend_status(rc: i32) -> Result<(), ConfigError> {
    if rc == 0 {
        Ok(())
    } else {
        Err(ConfigError::from_libgit2(rc))
    }
}

impl UnopenedBackend {
    pub fn as_ref(&self) -> BackendRef<'_> {
        self.0.as_ref()
    }

    /// Opens once, without repository-dependent includes. Memory constructors
    /// ignore repo. Repository-retaining custom/file constructors are not exposed
    /// by this phase type and require a lifetime-carrying owner when scheduled.
    pub fn open(self, level: ConfigLevel) -> Result<BackendBox, ConfigError> {
        BackendOpen.call(self, level)
    }
}

impl<'a, 'r> ConfigMut<'a, 'r> {
    /// Opens and transfers a backend into this configuration, optionally replacing
    /// the existing priority. The backend is consumed on failure too: C may already
    /// have opened it, so it cannot safely be returned to the construction phase.
    /// Successful registration is released by the config (or its last level view).
    ///
    /// Priorities must be in `0..i32::MAX`, as with `add_file`. A repository, when
    /// supplied, stays borrowed for the configuration's `'r`.
    ///
    /// Wraps: git_config_add_backend
    pub fn add_backend(
        &mut self,
        mut backend: UnopenedBackend,
        level: ConfigLevel,
        repository: Option<crate::libgit2::repository::repository::RepositoryRef<'r>>,
        force: bool,
    ) -> Result<(), ConfigError> {
        if !(0..i32::MAX).contains(&level.raw()) {
            set(
                ErrorClass::Invalid,
                c"configuration priority must be in 0..i32::MAX",
            );
            return Err(ConfigError::from_libgit2(ffi::GIT_EINVALID));
        }
        let p = backend.0.as_mut().as_mut_ptr();
        // SAFETY: the construction-phase owner has an initialized vtable. C calls
        // open unconditionally, so reject a missing slot before entering it.
        if unsafe { addr_of!((*p).open).read() }.is_none() {
            return Err(missing_slot("open"));
        }
        // SAFETY: config and backend are exclusive live allocations; the bounded
        // priority cannot overflow C's comparators. Repository is NULL or kept live
        // for 'r. C opens once, retaining backend only on success; on failure the
        // still-owned allocation is released by its matching policy here.
        let rc = unsafe {
            ffi::git_config_add_backend(
                self.as_mut_ptr(),
                p,
                level.raw(),
                repository.map_or(ptr::null(), |r| r.as_ptr()),
                i32::from(force),
            )
        };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // Only success transfers ownership; freeing this allocation now would
        // invalidate C's backend instance and cause a second free on config drop.
        let _ = backend.0.into_c();
        Ok(())
    }
}

/// One-time initialization callable; never accepts an already opened backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct BackendOpen;
impl BackendOpen {
    pub fn call(
        &self,
        mut backend: UnopenedBackend,
        level: ConfigLevel,
    ) -> Result<BackendBox, ConfigError> {
        let p = backend.0.as_mut().as_mut_ptr();
        // SAFETY: initialized allocation-specific vtable; missing slot is checked.
        let open = unsafe { addr_of!((*p).open).read() }.ok_or_else(|| missing_slot("open"))?;
        // SAFETY: construction phase owns the entire allocation exclusively; NULL
        // repo is supported, and only this consuming call can initialize it.
        backend_status(unsafe { open(p, level.raw(), ptr::null()) })?;
        Ok(backend.0)
    }
}

impl BackendFree {
    pub fn release(self, backend: BackendBox) {
        drop(backend);
    }
}

impl BackendRef<'_> {
    /// Field: git_config_backend.version
    pub fn version(&self) -> u32 {
        // SAFETY: initialized scalar under the shared handle.
        unsafe { addr_of!((*self.as_ptr()).version).read() }
    }
    /// C's integer flag, preserving noncanonical true values.
    /// Field: git_config_backend.readonly
    pub fn readonly_raw(&self) -> i32 {
        // SAFETY: initialized scalar under the shared handle.
        unsafe { addr_of!((*self.as_ptr()).readonly).read() }
    }
    pub fn is_readonly(&self) -> bool {
        self.readonly_raw() != 0
    }

    /// Identity only: registering configurations can die before shared backends.
    /// Field: git_config_backend.cfg
    pub fn config_identity(&self) -> Option<BackendConfigIdentity> {
        // SAFETY: reads the pointer slot only, never its possibly stale referent.
        NonNull::new(unsafe { addr_of!((*self.as_ptr()).cfg).read() }).map(BackendConfigIdentity)
    }
    /// Field: git_config_backend.open
    pub fn open_callback(&self) -> BackendOpen {
        BackendOpen
    }
    /// Field: git_config_backend.free
    pub fn release_policy(&self) -> BackendFree {
        BackendFree
    }
    /// A callable which can only consume a transaction, not unlock an unrelated
    /// backend or a borrowed allocation that has not been locked.
    /// Field: git_config_backend.unlock
    pub fn unlock_callback(&self) -> BackendUnlock {
        BackendUnlock
    }
}

impl BackendMut<'_> {
    /// Acquires a retained immutable entry, independent of the backend lifetime,
    /// through the backend's own get callback, as C dispatches it. A writable
    /// file backend rereads a changed file first and reports a failed reread
    /// without writing an entry.
    /// Field: git_config_backend.get
    pub fn get(&mut self, key: &CStr) -> Result<BackendEntryArc, ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: live initialized vtable; missing callback is checked.
        let get = unsafe { addr_of!((*p).get).read() }.ok_or_else(|| missing_slot("get"))?;
        let mut out = ptr::null_mut();
        // SAFETY: exclusive opened backend, readable key for this call, writable
        // out, which is read below only after the callback reports success.
        backend_status(unsafe { get(p, key.as_ptr(), &mut out) })?;
        // SAFETY: success acquires one reference with its matching free callback.
        Ok(unsafe { BackendEntryArc::from_c(out) }.expect("backend get returned NULL"))
    }

    /// Borrows this exclusive handle for the entire iterator lifetime. Entries
    /// lent by the iterator cannot alias a subsequent advance or backend write.
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::sys::config::BackendMut;
    /// fn alias(mut backend: BackendMut<'_>) {
    ///     let iter = backend.iterator().unwrap();
    ///     backend.set(c"a.b", c"new").unwrap();
    ///     drop(iter);
    /// }
    /// ```
    ///
    /// Field: git_config_backend.iterator
    pub fn iterator(&mut self) -> Result<IteratorBox<'_>, ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: initialized live vtable, with NULL checked.
        let iterator = unsafe { addr_of!((*p).iterator).read() };
        let iterator = iterator.ok_or_else(|| missing_slot("iterator"))?;
        let mut out = ptr::null_mut();
        // SAFETY: exclusive opened backend, writable output; result retains its
        // own entries and any borrowed backend remains frozen for this reborrow.
        backend_status(unsafe { iterator(&mut out, p) })?;
        // SAFETY: fresh complete iterator with its own matched free callback;
        // backend dependencies cannot die or be mutated until this owner dies.
        Ok(unsafe { IteratorBox::from_c(out) }.expect("backend iterator returned NULL"))
    }

    /// Immediately opens the new snapshot while its source is still borrowed.
    /// The snapshot stores a source pointer used only during that one-time open;
    /// subsequent get/iterator/free operations never dereference it.
    /// Field: git_config_backend.snapshot
    pub fn snapshot(&mut self) -> Result<BackendBox, ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: initialized live allocation-specific callback.
        let snapshot = unsafe { addr_of!((*p).snapshot).read() };
        let snapshot = snapshot.ok_or_else(|| missing_slot("snapshot"))?;
        let mut out = ptr::null_mut();
        // SAFETY: exclusive live source and writable output. C constructs a fresh
        // snapshot borrowing source only until its one-time open below completes.
        backend_status(unsafe { snapshot(&mut out, p) })?;
        // SAFETY: success transfers a complete snapshot allocation and matching
        // free callback; self remains live through its synchronous initialization.
        let snapshot = unsafe { BackendBox::from_c(out) };
        UnopenedBackend(snapshot.expect("snapshot returned NULL")).open(ConfigLevel::LOCAL)
    }

    /// Field: git_config_backend.set
    pub fn set(&mut self, key: &CStr, value: &CStr) -> Result<(), ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: initialized matching vtable; absent callback is rejected.
        let set = unsafe { addr_of!((*p).set).read() }.ok_or_else(|| missing_slot("set"))?;
        // SAFETY: exclusive opened backend, C reads/copies strings within the call.
        backend_status(unsafe { set(p, key.as_ptr(), value.as_ptr()) })
    }
    /// Field: git_config_backend.set_multivar
    pub fn set_multivar(
        &mut self,
        key: &CStr,
        regexp: &CStr,
        value: &CStr,
    ) -> Result<(), ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: initialized matching vtable; absent callback is rejected.
        let set = unsafe { addr_of!((*p).set_multivar).read() }
            .ok_or_else(|| missing_slot("set_multivar"))?;
        // SAFETY: exclusive opened backend; strings are readable for this call only.
        backend_status(unsafe { set(p, key.as_ptr(), regexp.as_ptr(), value.as_ptr()) })
    }
    /// Field: git_config_backend.del
    pub fn delete(&mut self, key: &CStr) -> Result<(), ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: initialized matching vtable; absent callback is rejected.
        let delete = unsafe { addr_of!((*p).del).read() }.ok_or_else(|| missing_slot("del"))?;
        // SAFETY: exclusive opened backend; key is read only during this call.
        backend_status(unsafe { delete(p, key.as_ptr()) })
    }
    /// Field: git_config_backend.del_multivar
    pub fn delete_multivar(&mut self, key: &CStr, regexp: &CStr) -> Result<(), ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: initialized matching vtable; absent callback is rejected.
        let delete = unsafe { addr_of!((*p).del_multivar).read() }
            .ok_or_else(|| missing_slot("del_multivar"))?;
        // SAFETY: exclusive opened backend; strings readable only for this call.
        backend_status(unsafe { delete(p, key.as_ptr(), regexp.as_ptr()) })
    }

    /// An exclusive transaction, not a mutex: file locks permit updates through
    /// the transaction and commit/rollback on finish. No Send/Sync is asserted.
    /// Field: git_config_backend.lock
    pub fn lock(&mut self) -> Result<BackendTransaction<'_>, ConfigError> {
        let p = self.as_mut_ptr();
        // SAFETY: initialized matching callbacks; check unlock before acquiring.
        let lock = unsafe { addr_of!((*p).lock).read() }.ok_or_else(|| missing_slot("lock"))?;
        // SAFETY: initialized matching callback under exclusive access.
        let unlock = unsafe { addr_of!((*p).unlock).read() };
        let unlock = unlock.ok_or_else(|| missing_slot("unlock"))?;
        // SAFETY: exclusive backend, no nested transaction can survive this borrow.
        backend_status(unsafe { lock(p) })?;
        Ok(BackendTransaction {
            backend: self.as_mut(),
            unlock: Some(unlock),
        })
    }
}

/// Exclusive transaction; dropping rolls back. It cannot expose an unrestricted
/// mutable backend handle (which would allow reentrant locking).
///
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendMut;
/// fn reenter(mut backend: BackendMut<'_>) {
///     let transaction = backend.lock().unwrap();
///     backend.lock().unwrap();
///     transaction.finish(false).unwrap();
/// }
/// ```
///
/// Handwritten
pub struct BackendTransaction<'a> {
    backend: BackendMut<'a>,
    unlock: Option<unsafe extern "C" fn(*mut ffi::git_config_backend, i32) -> i32>,
}
impl BackendTransaction<'_> {
    pub fn set(&mut self, key: &CStr, value: &CStr) -> Result<(), ConfigError> {
        self.backend.set(key, value)
    }
    pub fn set_multivar(
        &mut self,
        key: &CStr,
        regexp: &CStr,
        value: &CStr,
    ) -> Result<(), ConfigError> {
        self.backend.set_multivar(key, regexp, value)
    }
    pub fn delete(&mut self, key: &CStr) -> Result<(), ConfigError> {
        self.backend.delete(key)
    }
    pub fn delete_multivar(&mut self, key: &CStr, regexp: &CStr) -> Result<(), ConfigError> {
        self.backend.delete_multivar(key, regexp)
    }
    pub fn finish(self, commit: bool) -> Result<(), ConfigError> {
        BackendUnlock.call(self, commit)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BackendUnlock;
impl BackendUnlock {
    pub fn call(
        &self,
        mut transaction: BackendTransaction<'_>,
        commit: bool,
    ) -> Result<(), ConfigError> {
        let unlock = transaction
            .unlock
            .take()
            .expect("transaction already finished");
        // SAFETY: matching callback consumes this transaction's successful lock
        // once; source remains exclusively borrowed until after the call returns.
        backend_status(unsafe { unlock(transaction.backend.as_mut_ptr(), i32::from(commit)) })
    }
}
/// Rolls back an unfinished transaction. A failed rollback is discarded, as
/// `git_transaction_free` discards `git_config_unlock`'s status; call
/// [`finish`](BackendTransaction::finish) to observe it.
impl Drop for BackendTransaction<'_> {
    fn drop(&mut self) {
        if let Some(unlock) = self.unlock.take() {
            // SAFETY: matching callback rolls back the one successful lock; its
            // exclusive backend borrow is still live, and finish did not unlock.
            unsafe { unlock(self.backend.as_mut_ptr(), 0) };
        }
    }
}

impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}
impl fmt::Debug for UnopenedBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UnopenedBackend")
            .field(&self.as_ref())
            .finish()
    }
}
impl fmt::Debug for BackendTransaction<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackendTransaction")
            .field("backend", &self.backend.as_ref())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for BackendRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backend")
            .field("version", &self.version())
            .field("readonly", &self.readonly_raw())
            .field("config", &self.config_identity())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for BackendMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    // Initialized Rust-owned fixture for Rust-only Miri execution (no C init).
    fn fixture() -> BackendMemoryOptions<'static> {
        BackendMemoryOptions(
            ffi::git_config_backend_memory_options {
                version: ffi::GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION,
                backend_type: ptr::null(),
                origin_path: ptr::null(),
            },
            PhantomData,
        )
    }

    #[test]
    fn layout_and_pointer_seams() {
        assert_eq!(
            size_of::<BackendMemoryOptions>(),
            size_of::<ffi::git_config_backend_memory_options>()
        );
        assert_eq!(
            align_of::<BackendMemoryOptions>(),
            align_of::<ffi::git_config_backend_memory_options>()
        );
        assert_eq!(size_of::<BackendMemoryOptionsRef>(), size_of::<*const ()>());
        assert_eq!(size_of::<BackendMemoryOptionsMut>(), size_of::<*mut ()>());
        let options = fixture();
        let ptr = options.as_ref().as_ptr();
        // SAFETY: shared initialized fixture, both strings NULL, unchanged here.
        let borrowed = unsafe { BackendMemoryOptionsRef::from_ptr(ptr) }.unwrap();
        assert_eq!(
            borrowed.version(),
            ffi::GIT_CONFIG_BACKEND_MEMORY_OPTIONS_VERSION
        );
        // SAFETY: NULL is rejected before a handle is created.
        assert!(unsafe { BackendMemoryOptionsRef::from_ptr(ptr::null()) }.is_none());
    }

    #[test]
    fn rust_only_reborrows_and_clone() {
        let mut options = fixture();
        let mut m = options.as_mut();
        m.as_mut().set_backend_type(Some(c"label"));
        m.set_origin_path(Some(c"origin"));
        assert_eq!(m.as_ref().backend_type(), Some(c"label"));
        assert_eq!(m.as_ref().origin_path(), Some(c"origin"));
        m.set_version(u32::MAX);
        assert_eq!(m.as_ref().version(), u32::MAX);
        let cloned = options.clone();
        options.as_mut().set_backend_type(None);
        options.as_mut().set_origin_path(None);
        assert_eq!(options.as_ref().backend_type(), None);
        assert_eq!(cloned.as_ref().backend_type(), Some(c"label"));
        assert_eq!(cloned.as_ref().origin_path(), Some(c"origin"));
    }

    #[test]
    fn debug_and_thread_traits() {
        fn shared<T: Send + Sync>(_: &T) {}
        let mut options = fixture();
        shared(&options);
        shared(&options.as_ref());
        shared(&options.as_mut());
        assert_eq!(
            format!("{options:?}"),
            "BackendMemoryOptions { version: 1, backend_type: None, origin_path: None }"
        );
        options.as_mut().set_backend_type(Some(c"label"));
        options.as_mut().set_origin_path(Some(c""));
        let expected = r#"BackendMemoryOptions { version: 1, backend_type: Some("label"), origin_path: Some("") }"#;
        assert_eq!(format!("{:?}", options.as_ref()), expected);
        assert_eq!(format!("{:?}", options.as_mut()), expected);
    }

    mod backend_entries {
        use super::*;
        use std::cell::Cell;
        use std::rc::Rc;

        // C has no standalone entry constructor. This Rust-owned fixture has static
        // strings; acquired-reference fixtures below pair a Box with its callback.
        fn fixture() -> ffi::git_config_backend_entry {
            ffi::git_config_backend_entry {
                entry: ffi::git_config_entry {
                    name: c"test.name".as_ptr(),
                    value: ptr::null(),
                    backend_type: c"test".as_ptr(),
                    origin_path: ptr::null(),
                    include_depth: u32::MAX,
                    level: ffi::GIT_CONFIG_LEVEL_LOCAL,
                },
                free: Some(release_fixture),
            }
        }

        #[repr(C)]
        struct AcquiredFixture {
            entry: ffi::git_config_backend_entry,
            releases: Rc<Cell<usize>>,
            child: Option<BackendEntryArc>,
        }

        unsafe extern "C" fn release_fixture(ptr: *mut ffi::git_config_backend_entry) {
            // SAFETY: called once for the acquired fixture's Box, whose first field
            // is the backend entry. No pointer into it remains in use after release.
            let allocation = unsafe { Box::from_raw(ptr.cast::<AcquiredFixture>()) };
            allocation.releases.set(allocation.releases.get() + 1);
        }

        fn acquired(releases: Rc<Cell<usize>>) -> BackendEntryArc {
            acquired_with_child(releases, None)
        }

        fn acquired_with_child(
            releases: Rc<Cell<usize>>,
            child: Option<BackendEntryArc>,
        ) -> BackendEntryArc {
            let allocation = Box::new(AcquiredFixture {
                entry: fixture(),
                releases,
                child,
            });
            // SAFETY: Box has the entry at offset zero with immutable static strings;
            // release_fixture consumes this one reference with the matching allocator.
            unsafe { BackendEntryArc::from_c(Box::into_raw(allocation).cast()) }.unwrap()
        }

        #[test]
        fn layout_and_exclusive_projection() {
            assert_eq!(
                size_of::<BackendEntry>(),
                size_of::<ffi::git_config_backend_entry>()
            );
            assert_eq!(
                align_of::<BackendEntry>(),
                align_of::<ffi::git_config_backend_entry>()
            );
            assert_eq!(size_of::<BackendEntryRef>(), size_of::<*const ()>());
            assert_eq!(size_of::<BackendEntryMut>(), size_of::<*mut ()>());
            assert_eq!(
                core::mem::offset_of!(ffi::git_config_backend_entry, entry),
                0
            );
            let mut raw = fixture();
            // SAFETY: exclusive initialized fixture; strings are immutable and static.
            let mut m = unsafe { BackendEntryMut::from_ptr(&raw mut raw) }.unwrap();
            assert_eq!(m.entry_mut().as_ref().name(), Some(c"test.name"));
            assert_eq!(m.as_mut().entry_mut().as_ref().include_depth(), u32::MAX);
            let shared = m.as_ref();
            let copied = shared;
            assert_eq!(copied.entry().value(), None);
            assert_eq!(shared.entry().backend_type(), Some(c"test"));
            let _policy = shared.release_policy();
            // SAFETY: NULL is rejected without dereferencing.
            assert!(unsafe { BackendEntryRef::from_ptr(ptr::null_mut()) }.is_none());
            // SAFETY: NULL is rejected without dereferencing.
            assert!(unsafe { BackendEntryMut::from_ptr(ptr::null_mut()) }.is_none());
        }

        #[test]
        fn debug_prints_the_entry_only() {
            let expected = concat!(
                r#"BackendEntry { entry: Entry { name: Some("test.name"), value: None, "#,
                r#"backend_type: Some("test"), origin_path: None, level: ConfigLevel(5), "#,
                r#"include_depth: 4294967295 }, .. }"#
            );
            let mut raw = fixture();
            // SAFETY: a Rust-owned fixture with static strings, unchanged while the
            // handle lives; nothing adopts or releases it.
            let shared = unsafe { BackendEntryRef::from_ptr(&raw mut raw) }.unwrap();
            assert_eq!(format!("{shared:?}"), expected);
            // SAFETY: as above, after the shared handle's last use; exclusive.
            let mut exclusive = unsafe { BackendEntryMut::from_ptr(&raw mut raw) }.unwrap();
            assert_eq!(format!("{exclusive:?}"), expected);
            assert_eq!(format!("{:?}", exclusive.as_mut()), expected);
            let value = BackendEntry(fixture(), PhantomData);
            assert_eq!(format!("{value:?}"), expected);
            assert_eq!(format!("{:?}", value.as_ref()), expected);
        }

        #[test]
        #[should_panic(expected = "backend entry lacks its release callback")]
        fn release_policy_rejects_missing_callback() {
            let mut raw = fixture();
            raw.free = None;
            // SAFETY: a Rust-owned fixture with static strings; the check reads only
            // the callback slot and nothing adopts or releases the entry.
            let entry = unsafe { BackendEntryRef::from_ptr(&raw mut raw) }.unwrap();
            let _ = entry.release_policy();
        }

        #[test]
        #[cfg_attr(miri, ignore = "release calls C; projections tested separately")]
        fn acquired_callback_and_owner_roundtrip() {
            let releases = Rc::new(Cell::new(0));
            let owner = acquired(releases.clone());
            let address = owner.as_c_ptr();
            let public = BackendEntry::into_entry(owner);
            assert_eq!(
                public.as_c_ptr().cast::<ffi::git_config_backend_entry>(),
                address
            );
            assert_eq!(releases.get(), 0);
            let owner = BackendEntry::from_entry(public);
            assert_eq!(owner.as_c_ptr(), address);
            let policy = owner.as_ref().release_policy();
            policy.release(owner);
            assert_eq!(releases.get(), 1);
            drop(acquired(releases.clone()));
            assert_eq!(releases.get(), 2);
            drop(BackendEntry::into_entry(acquired(releases.clone())));
            assert_eq!(releases.get(), 3);
            let child = acquired(releases.clone());
            let parent = acquired_with_child(releases.clone(), Some(child));
            // Freeing the payload reenters C's release dispatch for another owner.
            // Each reference is consumed once; neither callback retains a borrow.
            BackendEntryFree.release(parent);
            assert_eq!(releases.get(), 5);
        }
    }

    mod iterators {
        use super::*;
        use std::cell::Cell;
        use std::rc::Rc;

        #[repr(C)]
        struct Fixture {
            base: ffi::git_config_iterator,
            entry: ffi::git_config_backend_entry,
            calls: usize,
            releases: Rc<Cell<usize>>,
            child: Option<IteratorBox<'static>>,
        }

        fn fixture(releases: Rc<Cell<usize>>) -> Fixture {
            Fixture {
                base: ffi::git_config_iterator {
                    backend: ptr::null_mut(),
                    flags: 0,
                    next: Some(next),
                    free: Some(free),
                },
                entry: ffi::git_config_backend_entry {
                    entry: ffi::git_config_entry {
                        name: c"fixture.key".as_ptr(),
                        value: ptr::null(),
                        backend_type: c"fixture".as_ptr(),
                        origin_path: ptr::null(),
                        include_depth: 0,
                        level: ffi::GIT_CONFIG_LEVEL_LOCAL,
                    },
                    free: None, // Borrowed-only fixture, never acquired or released.
                },
                calls: 0,
                releases,
                child: None,
            }
        }

        unsafe extern "C" fn next(
            out: *mut *mut ffi::git_config_backend_entry,
            p: *mut ffi::git_config_iterator,
        ) -> i32 {
            let p = p.cast::<Fixture>();
            // SAFETY: our fixture always has its initialized base at offset zero;
            // callback receives exclusive access, and out is a writable output slot.
            unsafe {
                let calls = addr_of_mut!((*p).calls);
                let n = calls.read();
                calls.write(n + 1);
                if n > 0 {
                    return ffi::GIT_ITEROVER;
                }
                out.write(addr_of_mut!((*p).entry));
            }
            0
        }

        unsafe extern "C" fn free(p: *mut ffi::git_config_iterator) {
            // SAFETY: owner adoption below transfers precisely this Box allocation,
            // with its first field at offset zero. Dispatch runs once, with no live
            // handle or entry remaining; child release may reenter C independently.
            let value = unsafe { Box::from_raw(p.cast::<Fixture>()) };
            value.releases.set(value.releases.get() + 1);
        }

        fn owned(
            releases: Rc<Cell<usize>>,
            child: Option<IteratorBox<'static>>,
        ) -> IteratorBox<'static> {
            let mut value = fixture(releases);
            value.child = child;
            // SAFETY: one initialized uniquely owned complete enclosing Box with
            // matching free callback and static entry strings, no config dependency.
            unsafe { IteratorBox::from_c(Box::into_raw(Box::new(value)).cast()) }.unwrap()
        }

        #[test]
        fn iterator_layout_and_borrowed_rust_callback() {
            assert_eq!(size_of::<Iterator>(), size_of::<ffi::git_config_iterator>());
            assert_eq!(
                align_of::<Iterator>(),
                align_of::<ffi::git_config_iterator>()
            );
            assert_eq!(size_of::<IteratorRef>(), size_of::<*const ()>());
            assert_eq!(size_of::<IteratorMut>(), size_of::<*mut ()>());
            assert_eq!(core::mem::offset_of!(Fixture, base), 0);
            let mut value = fixture(Rc::new(Cell::new(0)));
            let p = NonNull::from(&mut value).cast::<Iterator<'static>>();
            // SAFETY: exclusively borrowed initialized Fixture with matching
            // callbacks; static strings; no other access to value during this borrow.
            let mut m = IteratorMut(IteratorRef(unsafe { CBorrowedPtr::new(p) }), PhantomData);
            m.as_mut().set_flags(u32::MAX);
            assert_eq!(m.as_ref().flags(), u32::MAX);
            assert!(m.as_ref().backend().is_none());
            let callback = m.as_ref().next_callback();
            assert_eq!(
                callback.call(&mut m).unwrap().unwrap().entry().name(),
                Some(c"fixture.key")
            );
            assert!(m.advance().unwrap().is_none());
            let expected = "Iterator { flags: 4294967295, backend: None, .. }";
            assert_eq!(format!("{m:?}"), expected);
            assert_eq!(format!("{:?}", m.as_ref()), expected);
            // SAFETY: NULL is rejected without dereference.
            assert!(unsafe { IteratorRef::from_ptr(ptr::null_mut()) }.is_none());
            // Only field access, not callback invocation: standalone base has no
            // enclosing Fixture and deliberately is never adopted as an owner.
            let mut base = Iterator(value.base, PhantomData);
            assert_eq!(format!("{base:?}"), expected);
            base.as_mut().set_flags(0);
            assert_eq!(base.as_ref().flags(), 0);
        }

        /// Corrected behavior: C's regex path calls strlen(NULL) on the first flag.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2 and PCRE2")]
        fn multivar_regex_skips_valueless() {
            use crate::libgit2::config::config::Config;
            use std::ffi::CString;
            let file = std::env::temp_dir()
                .join(format!("crustify-filter-correction-{}", std::process::id()));
            std::fs::write(&file, b"[a]\nk\nk =\nk = value\n").unwrap();
            let path = CString::new(file.to_str().unwrap()).unwrap();
            let config = Config::open(&path).unwrap();
            let mut iter = config
                .as_ref()
                .multivar_iterator(c"a.k", Some(c"^$"))
                .unwrap();
            assert_eq!(
                iter.as_mut().advance().unwrap().unwrap().entry().value(),
                Some(c"")
            );
            assert!(iter.as_mut().advance().unwrap().is_none());
            let bad = config
                .as_ref()
                .multivar_iterator(c"a.k", Some(c"["))
                .unwrap_err();
            assert_eq!(bad.raw_code(), ffi::GIT_EINVALIDSPEC);
            // C normalizes names before compiling the regex: preserve that precedence.
            assert_eq!(
                config
                    .as_ref()
                    .multivar_iterator(c"invalid", Some(c"["))
                    .unwrap_err()
                    .raw_code(),
                ffi::GIT_EINVALIDSPEC
            );
            drop(iter);
            drop(config);
            std::fs::remove_file(file).unwrap();
        }

        #[test]
        #[cfg_attr(miri, ignore = "Drop dispatch calls C")]
        fn iterator_owned_release_and_reentry() {
            let releases = Rc::new(Cell::new(0));
            let mut first = owned(releases.clone(), None);
            let policy = first.as_ref().release_policy();
            assert!(first.as_mut().advance().unwrap().is_some());
            let ptr = first.into_c();
            // SAFETY: ownership roundtrip of a complete matching allocation; nothing
            // accesses the surrendered pointer, and dependencies are still static.
            let first = unsafe { IteratorBox::from_c(ptr) }.unwrap();
            policy.release(first);
            assert_eq!(releases.get(), 1);
            let child = owned(releases.clone(), None);
            let parent = owned(releases.clone(), Some(child));
            policy.release(parent);
            assert_eq!(releases.get(), 3);
            // SAFETY: NULL adoption never creates an owner or calls its destructor.
            assert!(unsafe { IteratorBox::from_c(ptr::null_mut()) }.is_none());
        }
    }

    mod backends {
        use super::*;
        use std::cell::Cell;
        use std::rc::Rc;

        #[derive(Default)]
        struct State {
            locked: Cell<bool>,
            writes: Cell<usize>,
            commits: Cell<usize>,
            rollbacks: Cell<usize>,
            frees: Cell<usize>,
            fail_rollback: Cell<bool>,
            opens: Cell<usize>,
            open_status: Cell<i32>,
        }
        #[repr(C)]
        struct Mock {
            base: ffi::git_config_backend,
            state: Rc<State>,
        }
        unsafe fn state<'a>(p: *mut ffi::git_config_backend) -> &'a State {
            // SAFETY: mock callbacks are installed only on complete Mock Boxes. The
            // reference covers the Rust-owned payload, not the projected C base.
            unsafe { &*addr_of!((*p.cast::<Mock>()).state) }
        }
        unsafe extern "C" fn lock(p: *mut ffi::git_config_backend) -> i32 {
            // SAFETY: matched Mock callback, allocation live for this call.
            let s = unsafe { state(p) };
            assert!(!s.locked.replace(true));
            0
        }
        unsafe extern "C" fn unlock(p: *mut ffi::git_config_backend, commit: i32) -> i32 {
            // SAFETY: matched Mock callback, allocation live for this call.
            let s = unsafe { state(p) };
            assert!(s.locked.replace(false));
            let counter = if commit != 0 {
                &s.commits
            } else {
                &s.rollbacks
            };
            counter.set(counter.get() + 1);
            if commit == 0 && s.fail_rollback.get() {
                -1
            } else {
                0
            }
        }
        unsafe extern "C" fn set(
            p: *mut ffi::git_config_backend,
            _key: *const core::ffi::c_char,
            _value: *const core::ffi::c_char,
        ) -> i32 {
            // SAFETY: matched Mock callback, allocation live for this call.
            let s = unsafe { state(p) };
            s.writes.set(s.writes.get() + 1);
            0
        }
        unsafe extern "C" fn set_multivar(
            p: *mut ffi::git_config_backend,
            key: *const core::ffi::c_char,
            _regexp: *const core::ffi::c_char,
            value: *const core::ffi::c_char,
        ) -> i32 {
            // SAFETY: forwards the same matched allocation and unused string inputs.
            unsafe { set(p, key, value) }
        }
        unsafe extern "C" fn delete(
            p: *mut ffi::git_config_backend,
            key: *const core::ffi::c_char,
        ) -> i32 {
            // SAFETY: forwards the same matched allocation and unused key input.
            unsafe { set(p, key, ptr::null()) }
        }
        unsafe extern "C" fn delete_multivar(
            p: *mut ffi::git_config_backend,
            key: *const core::ffi::c_char,
            _regexp: *const core::ffi::c_char,
        ) -> i32 {
            // SAFETY: forwards the same matched allocation and unused key input.
            unsafe { delete(p, key) }
        }
        unsafe extern "C" fn free(p: *mut ffi::git_config_backend) {
            // SAFETY: called once with the sole owner of the exact offset-zero Box.
            let mock = unsafe { Box::from_raw(p.cast::<Mock>()) };
            assert!(!mock.state.locked.get());
            mock.state.frees.set(mock.state.frees.get() + 1);
        }
        fn fixture() -> (BackendBox, Rc<State>) {
            let state = Rc::new(State::default());
            let raw = ffi::git_config_backend {
                version: 1,
                readonly: 0,
                lock: Some(lock),
                unlock: Some(unlock),
                set: Some(set),
                set_multivar: Some(set_multivar),
                del: Some(delete),
                del_multivar: Some(delete_multivar),
                free: Some(free),
                ..ffi::git_config_backend::default()
            };
            let mock = Box::new(Mock {
                base: raw,
                state: state.clone(),
            });
            // SAFETY: fresh complete Mock allocation, offset-zero initialized ABI
            // base with matching callbacks; no external borrow, strings not retained.
            let backend = unsafe { BackendBox::from_c(Box::into_raw(mock).cast()) };
            (backend.unwrap(), state)
        }
        unsafe extern "C" fn registration_open(
            p: *mut ffi::git_config_backend,
            _level: ffi::git_config_level_t,
            _repo: *const ffi::git_repository,
        ) -> i32 {
            // SAFETY: only complete Mock allocations install this callback; its
            // Rust-owned state is live, and the construction phase is exclusive.
            let s = unsafe { state(p) };
            s.opens.set(s.opens.get() + 1);
            if s.open_status.get() < 0 {
                crate::libgit2::util::sys::errors::set(ErrorClass::Config, c"mock open failure");
            }
            s.open_status.get()
        }
        fn pending() -> (UnopenedBackend, Rc<State>) {
            let (mut owner, state) = fixture();
            // SAFETY: exclusive complete Mock fixture, its compatible construction
            // callback is installed before any open or registration is possible.
            unsafe {
                addr_of_mut!((*owner.as_mut().as_mut_ptr()).open).write(Some(registration_open))
            };
            (UnopenedBackend(owner), state)
        }
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2 registration")]
        fn backend_registration_open_transfer_errors_and_release_counts() {
            use crate::libgit2::config::config::Config;
            let mut config = Config::new().unwrap();
            let (backend, kept) = pending();
            config
                .as_mut()
                .add_backend(backend, ConfigLevel::LOCAL, None, false)
                .unwrap();
            assert_eq!(kept.opens.get(), 1);
            assert_eq!(kept.frees.get(), 0);
            let (backend, duplicate) = pending();
            let error = config
                .as_mut()
                .add_backend(backend, ConfigLevel::LOCAL, None, false)
                .unwrap_err();
            assert_eq!(error.raw_code(), ffi::GIT_EEXISTS);
            assert_eq!((duplicate.opens.get(), duplicate.frees.get()), (1, 1));
            let (backend, failed) = pending();
            failed.open_status.set(-7);
            let error = config
                .as_mut()
                .add_backend(backend, ConfigLevel::LOCAL, None, true)
                .unwrap_err();
            assert_eq!(error.raw_code(), -7);
            assert_eq!((failed.opens.get(), failed.frees.get()), (1, 1));
            assert_eq!(kept.frees.get(), 0); // failed open precedes replacement
            let (backend, replacement) = pending();
            config
                .as_mut()
                .add_backend(backend, ConfigLevel::LOCAL, None, true)
                .unwrap();
            assert_eq!(kept.frees.get(), 1);
            drop(config);
            assert_eq!(replacement.frees.get(), 1);
            let mut config = Config::new().unwrap();
            let (backend, missing) = fixture(); // free is valid, open is absent
            let error = config
                .as_mut()
                .add_backend(UnopenedBackend(backend), ConfigLevel::LOCAL, None, false)
                .unwrap_err();
            assert_eq!(error.message(), c"config backend does not implement 'open'");
            assert_eq!(missing.frees.get(), 1);
            let (mut backend, bad_version) = pending();
            // SAFETY: fixture is uniquely owned; version refusal precedes dispatch.
            unsafe { addr_of_mut!((*backend.0.as_mut().as_mut_ptr()).version).write(2) };
            assert!(
                config
                    .as_mut()
                    .add_backend(backend, ConfigLevel::LOCAL, None, false)
                    .is_err()
            );
            assert_eq!((bad_version.opens.get(), bad_version.frees.get()), (0, 1));
            let (backend, bad_priority) = pending();
            assert!(
                config
                    .as_mut()
                    .add_backend(backend, ConfigLevel::HIGHEST, None, false)
                    .is_err()
            );
            assert_eq!((bad_priority.opens.get(), bad_priority.frees.get()), (0, 1));
        }
        #[test]
        #[cfg_attr(miri, ignore = "installs C allocator and calls registration")]
        fn backend_registration_allocator_failures_release_owned_inputs() {
            use crate::libgit2::config::config::{Config, unit_tests::*};
            isolated(
                concat!(
                    module_path!(),
                    "::backend_registration_allocator_failures_release_owned_inputs"
                ),
                || {
                    install_refusing();
                    let mut config = Config::new().unwrap();
                    // Warm thread error storage before restricting allocations.
                    assert!(config.as_ref().get_i32(c"a.b").is_err());
                    for granted in [0, 1] {
                        // instance calloc, then backend_entry malloc
                        let (backend, state) = pending();
                        ALLOWED_ALLOCATIONS.store(granted, std::sync::atomic::Ordering::SeqCst);
                        let result =
                            config
                                .as_mut()
                                .add_backend(backend, ConfigLevel::LOCAL, None, false);
                        ALLOWED_ALLOCATIONS.store(usize::MAX, std::sync::atomic::Ordering::SeqCst);
                        assert_eq!(result.unwrap_err().class(), Some(ErrorClass::NoMemory));
                        assert_eq!((state.opens.get(), state.frees.get()), (1, 1));
                    }
                    for raw in 0..8 {
                        config
                            .as_mut()
                            .add_backend(pending().0, ConfigLevel::from_raw(raw), None, false)
                            .unwrap();
                    }
                    let (backend, state) = pending();
                    // Instance/entry allocate; readers vector growth is refused. Do not
                    // refuse writers after readers insertion: C leaves a dangling entry.
                    ALLOWED_ALLOCATIONS.store(2, std::sync::atomic::Ordering::SeqCst);
                    let result =
                        config
                            .as_mut()
                            .add_backend(backend, ConfigLevel::from_raw(8), None, false);
                    ALLOWED_ALLOCATIONS.store(usize::MAX, std::sync::atomic::Ordering::SeqCst);
                    assert_eq!(result.unwrap_err().class(), Some(ErrorClass::NoMemory));
                    assert_eq!(state.frees.get(), 1);
                },
            );
        }
        #[test]
        fn backend_transactions_release_once_and_reborrow() {
            let (mut backend, state) = fixture();
            let unlock = backend.as_ref().unlock_callback();
            {
                let mut b = backend.as_mut();
                let mut transaction = b.lock().unwrap();
                transaction.set(c"a.b", c"v").unwrap();
                transaction.set_multivar(c"a.b", c".*", c"v").unwrap();
                transaction.delete(c"a.b").unwrap();
                transaction.delete_multivar(c"a.b", c".*").unwrap();
                unlock.call(transaction, true).unwrap();
                b.lock().unwrap().finish(false).unwrap();
                drop(b.as_mut().lock().unwrap());
            }
            assert_eq!(state.writes.get(), 4);
            assert_eq!(state.commits.get(), 1);
            assert_eq!(state.rollbacks.get(), 2);
            assert_eq!(state.frees.get(), 0);
            backend.as_ref().release_policy().release(backend);
            assert_eq!(state.frees.get(), 1);
        }
        #[test]
        fn failed_rollback_on_drop_is_discarded() {
            let (mut backend, state) = fixture();
            state.fail_rollback.set(true);
            let mut b = backend.as_mut();
            let transaction = b.lock().unwrap();
            assert!(
                format!("{transaction:?}").starts_with("BackendTransaction { backend: Backend {")
            );
            drop(transaction);
            assert_eq!(state.rollbacks.get(), 1);
            assert!(!state.locked.get());
            // Unwinding past an open transaction rolls back without a second panic.
            let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _transaction = b.lock().unwrap();
                panic!("caller failure");
            }));
            assert!(unwound.is_err());
            assert_eq!(state.rollbacks.get(), 2);
            // finish still reports the status that drop discards (the error's
            // construction reads C's thread error, which Miri cannot call).
            if !cfg!(miri) {
                assert!(b.lock().unwrap().finish(false).is_err());
                assert_eq!(state.rollbacks.get(), 3);
            }
            assert!(b.lock().unwrap().finish(true).is_ok());
            assert_eq!(state.commits.get(), 1);
            drop(backend);
            assert_eq!(state.frees.get(), 1);
        }
        #[test]
        fn backend_rust_layout_and_stale_identity_are_not_dereferenced() {
            assert_eq!(
                core::mem::size_of::<Backend>(),
                core::mem::size_of::<ffi::git_config_backend>()
            );
            assert_eq!(
                core::mem::align_of::<Backend>(),
                core::mem::align_of::<ffi::git_config_backend>()
            );
            let config = NonNull::<ffi::git_config>::dangling().as_ptr();
            let mut backend = Backend(
                ffi::git_config_backend {
                    version: u32::MAX,
                    readonly: -7,
                    cfg: config,
                    ..Default::default()
                },
                PhantomData,
            );
            assert!(backend.as_ref().config_identity().is_some());
            assert_eq!(backend.as_ref().version(), u32::MAX);
            assert_eq!(backend.as_mut().as_ref().readonly_raw(), -7);
            assert!(backend.as_ref().is_readonly());
            let r = backend.as_ref();
            assert_eq!(r.config_identity(), r.config_identity());
            assert!(format!("{r:?}").contains("-7"));
            assert!(format!("{:?}", backend.as_mut()).contains("-7"));
            assert_eq!(format!("{backend:?}"), format!("{:?}", backend.as_ref()));
        }
        #[test]
        #[cfg_attr(miri, ignore = "error construction calls C")]
        fn backend_missing_slots_reject_before_ffi_dispatch() {
            let mut backend = Backend::default();
            assert_eq!(backend.as_ref().version(), ffi::GIT_CONFIG_BACKEND_VERSION);
            assert!(!backend.as_ref().is_readonly());
            assert!(backend.as_ref().config_identity().is_none());
            let mut b = backend.as_mut();
            // Each refusal names its slot rather than reporting an earlier failure.
            use crate::libgit2::util::sys::errors;
            let refused = |result: Result<(), ConfigError>, slot: &str| {
                let error = result.unwrap_err();
                assert_eq!(error.raw_code(), ffi::GIT_ERROR);
                assert_eq!(error.class(), Some(ErrorClass::Config));
                let expected = format!("config backend does not implement '{slot}'");
                assert_eq!(error.message().to_str().unwrap(), expected);
            };
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.get(c"a.b").map(drop), "get");
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.iterator().map(drop), "iterator");
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.snapshot().map(drop), "snapshot");
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.set(c"a.b", c"v"), "set");
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.set_multivar(c"a.b", c".*", c"v"), "set_multivar");
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.delete(c"a.b"), "del");
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.delete_multivar(c"a.b", c".*"), "del_multivar");
            errors::set(ErrorClass::Odb, c"an earlier, unrelated failure");
            refused(b.lock().map(drop), "lock");
            let p = b.as_mut_ptr();
            // SAFETY: exclusive inert template; changing its scalar leaves every
            // field initialized and permits testing the absent read-only get slot.
            unsafe { addr_of_mut!((*p).readonly).write(1) };
            assert!(b.get(c"a.b").is_err());
            let (mut backend, state) = fixture();
            let p = backend.as_mut().as_mut_ptr();
            // SAFETY: exclusive mock fixture, no lock held; remove unlock to check
            // rejection before the otherwise valid lock callback can mutate state.
            unsafe { addr_of_mut!((*p).unlock).write(None) };
            let error = backend.as_mut().lock().unwrap_err();
            assert_eq!(
                error.message(),
                c"config backend does not implement 'unlock'"
            );
            assert!(!state.locked.get());
            drop(backend);
            assert_eq!(state.frees.get(), 1);
            let (backend, state) = fixture();
            let pending = UnopenedBackend(backend);
            assert!(format!("{pending:?}").starts_with("UnopenedBackend(Backend { version: 1,"));
            let error = pending.open(ConfigLevel::LOCAL).unwrap_err();
            assert_eq!(error.message(), c"config backend does not implement 'open'");
            assert_eq!(state.frees.get(), 1);
        }
    }
}
