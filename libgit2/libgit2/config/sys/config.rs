//! Safe wrappers over `include/git2/sys/config.h`.
//!
//! Memory backend options are caller-owned values. Their nullable strings are
//! borrowed, never freed by C. `config_backend_new` copies both strings; neither
//! `git_config_backend_from_string` nor `git_config_backend_from_values` retains
//! the options. There is no C allocation, destruction or cloning primitive for
//! the options themselves. A Rust clone simply shares the immutable string borrows,
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

use ffibox::{CArc, CBorrowedPtr, CCell, CDrop, define_ctype};

use crate::libgit2::config::config::{ConfigError, EntryArc, EntryMut, EntryRef};

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
    /// NULL. This raw seam is temporary until `git_config_backend` is wrapped;
    /// the pointer must not be dereferenced after the iterator's dependencies die.
    ///
    /// Field: git_config_iterator.backend
    pub fn backend_raw(&self) -> Option<NonNull<ffi::git_config_backend>> {
        // SAFETY: reads only the initialized pointer slot, not its referent.
        NonNull::new(unsafe { addr_of!((*self.as_ptr()).backend).read() })
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

    /// Replaces a stored backend borrow without freeing the old one. Temporary
    /// raw seam for the unavailable higher-layer backend wrapper.
    ///
    /// # Safety
    /// The new backend is NULL or live for `'c` (including destruction), and
    /// replacing it preserves the enclosing iterator's next/free callback
    /// invariants. No operation may mutate/free it incompatibly while retained.
    /// In particular, a custom iterator may require its original backend.
    pub unsafe fn set_backend_raw(&mut self, backend: Option<NonNull<ffi::git_config_backend>>) {
        let backend = backend.map_or(ptr::null_mut(), NonNull::as_ptr);
        // SAFETY: exclusive access to the slot; the caller guarantees keepalive
        // and compatibility with the enclosing allocation's callbacks.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).backend).write(backend) }
    }

    /// Advances the allocation's inline callback. `None` is C's ITEROVER; other
    /// errors preserve the code and copied thread-local error. A subsequent call
    /// after an error is allowed (C can proceed to a different backend).
    ///
    /// Wraps: git_config_next
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

impl<'a, 'r> crate::libgit2::config::config::ConfigRef<'a, 'r> {
    /// Creates an iterator borrowing this config, without retaining a reference.
    ///
    /// Wraps: git_config_iterator_new
    pub fn iterator(&self) -> Result<IteratorBox<'a>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: this handle keeps config/repository live for 'a; writable out
        // slot; C constructs a fresh complete allocation or cleans up failure.
        let rc = unsafe { ffi::git_config_iterator_new(&mut out, self.as_ptr()) };
        if rc != 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: success wrote a fresh all-iterator owned only by out, paired
        // with its own free callback; it borrows the config only for 'a.
        let iter = unsafe { IteratorBox::from_c(out) };
        Ok(iter.expect("successful iterator constructor returned NULL"))
    }

    /// Iterates keys matching C's regex syntax; `None` means all keys. The regex
    /// is compiled/copied during construction, not retained as a Rust borrow.
    ///
    /// Wraps: git_config_iterator_glob_new
    pub fn iterator_matching(&self, regexp: Option<&CStr>) -> Result<IteratorBox<'a>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: live config and writable out; regexp is NUL-terminated and
        // borrowed only for the call. C frees failed regex construction.
        let rc = unsafe {
            ffi::git_config_iterator_glob_new(
                &mut out,
                self.as_ptr(),
                regexp.map_or(ptr::null(), CStr::as_ptr),
            )
        };
        if rc != 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: success wrote a fresh glob (or, without a regex, all) iterator
        // owned only by out, with its own free callback, borrowing config for 'a.
        let iter = unsafe { IteratorBox::from_c(out) };
        Ok(iter.expect("successful iterator constructor returned NULL"))
    }

    /// Iterates all values for the normalized name, optionally regex-filtered.
    /// Name and regexp are copied/compiled, so their inputs may die immediately.
    ///
    /// With a regex, C's name-selection iterator is retained but value matching
    /// uses the same PCRE2 engine in a Rust-owned enclosing allocation. C calls
    /// strlen(NULL) on valueless entries and leaks its name on regex failure;
    /// the filter instead skips valueless entries and cleans up failed regexes.
    /// Empty values remain distinct and are matched normally. This corrects
    /// undefined C behavior without changing matching of non-null values.
    /// C's existing outer-allocation-failure leak without a regex remains.
    ///
    /// Wraps: git_config_multivar_iterator_new
    pub fn multivar_iterator(
        &self,
        name: &CStr,
        regexp: Option<&CStr>,
    ) -> Result<IteratorBox<'a>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: live config; writable out; C copies the name and returns a
        // fresh matching-callback allocation. No regex: valueless entries are
        // safe here, and failed normalization releases its partial construction.
        let rc = unsafe {
            ffi::git_config_multivar_iterator_new(
                &mut out,
                self.as_ptr(),
                name.as_ptr(),
                ptr::null(),
            )
        };
        if rc != 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: success wrote a fresh multivar iterator owned only by out, with
        // its own free callback; it and its inner iterator borrow config for 'a.
        let inner = unsafe { IteratorBox::from_c(out) }
            .expect("successful iterator constructor returned NULL");
        match regexp {
            None => Ok(inner),
            Some(regexp) => value_filter::new(inner, regexp),
        }
    }
}

// A narrow workaround for C's valueless multivar regex UB. The campaign builds
// libgit2 with USE_REGEX=pcre2; these bindings link the same system PCRE2 library,
// use the same options and preserve arbitrary pattern/value bytes (not UTF-8).
mod value_filter {
    use super::*;
    use crate::libgit2::util::errors::ErrorClass;
    use crate::libgit2::util::sys::errors::{set, set_oom};
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

    pub(super) fn new<'c>(
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
            .field("backend", &self.backend_raw())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for IteratorMut<'_, '_> {
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
            assert!(m.as_ref().backend_raw().is_none());
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

        /// Constructors fail cleanly when libgit2's allocator refuses: the all
        /// iterator's (and glob's) only allocation, and the multivar name copy
        /// after C has built both iterators. C's leak when the multivar shell
        /// itself is refused (`config.c:1139`, its second allocation) is skipped.
        mod allocation_failures {
            use super::*;
            use crate::libgit2::config::config::{Config, ConfigRef};
            use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
            use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
            use core::ffi::c_void;
            use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

            const CHILD: &str = "CRUSTIFY_UNIT_ITERATOR_OOM_CHILD";
            static ALLOWED: AtomicUsize = AtomicUsize::new(usize::MAX);

            fn refuse() -> bool {
                let mut remaining = ALLOWED.load(SeqCst);
                loop {
                    match remaining {
                        usize::MAX => return false,
                        0 => return true,
                        n => match ALLOWED.compare_exchange(n, n - 1, SeqCst, SeqCst) {
                            Ok(_) => return false,
                            Err(current) => remaining = current,
                        },
                    }
                }
            }

            // SAFETY: libc's allocator, declared with its C signatures.
            unsafe extern "C" {
                fn malloc(n: usize) -> *mut c_void;
                fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
                fn free(p: *mut c_void);
            }

            /// libc's allocator, granting `ALLOWED` more allocations.
            struct Counting;

            // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
            unsafe impl Alloc for Counting {
                fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                    if refuse() {
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
                    if refuse() {
                        return None;
                    }
                    let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
                    // SAFETY: NULL or a live libc block the caller gives up on success.
                    NonNull::new(unsafe { realloc(p, len) }.cast())
                }
                unsafe fn free(p: NonNull<u8>) {
                    // SAFETY: a live libc block the caller gives up.
                    unsafe { free(p.as_ptr().cast()) }
                }
            }

            #[test]
            #[cfg_attr(miri, ignore = "installs a libgit2 allocator")]
            fn constructors_report_allocation_failure() {
                let path = concat!(module_path!(), "::constructors_report_allocation_failure");
                let name = path.split_once("::").map_or(path, |(_, rest)| rest);
                if std::env::var_os(CHILD).is_none() {
                    // A fresh process, so the allocator is set before libgit2 starts.
                    let out = std::process::Command::new(std::env::current_exe().unwrap())
                        .args([name, "--exact", "--test-threads=1", "--nocapture"])
                        .env(CHILD, "1")
                        .output()
                        .unwrap();
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    assert!(
                        out.status.success() && stdout.contains("1 passed"),
                        "{stdout}\n{}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    return;
                }
                let allocator = Allocator::new::<Counting>();
                // SAFETY: `Allocator` is transparent over a fully set `git_allocator`;
                // libgit2 copies it before anything is allocated.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_ALLOCATOR as core::ffi::c_int,
                        allocator.as_ref().as_ptr(),
                    )
                };
                assert_eq!(rc, 0);
                let config = Config::new().unwrap();
                // Warm the thread's error state before limiting allocations.
                let missing = config.as_ref().multivar_iterator(c"invalid", None);
                assert_eq!(missing.unwrap_err().raw_code(), ffi::GIT_EINVALIDSPEC);
                type Build =
                    for<'a> fn(ConfigRef<'a, 'static>) -> Result<IteratorBox<'a>, ConfigError>;
                let cases: [(usize, Build); 6] = [
                    (0, |c| c.iterator()),
                    (0, |c| c.iterator_matching(None)),
                    (0, |c| c.iterator_matching(Some(c"^a"))),
                    (0, |c| c.multivar_iterator(c"a.b", None)),
                    (2, |c| c.multivar_iterator(c"a.b", None)),
                    (2, |c| c.multivar_iterator(c"a.b", Some(c"^a"))),
                ];
                for (granted, build) in cases {
                    ALLOWED.store(granted, SeqCst);
                    let result = build(config.as_ref());
                    ALLOWED.store(usize::MAX, SeqCst);
                    let error = result.unwrap_err();
                    assert_eq!(error.code(), Some(ErrorCode::Generic));
                    assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                    let mut iter = build(config.as_ref()).unwrap();
                    assert!(iter.as_mut().advance().unwrap().is_none());
                }
            }
        }
    }
}
