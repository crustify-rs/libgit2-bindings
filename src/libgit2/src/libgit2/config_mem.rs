//! The options of `src/libgit2/config_mem.c`'s in-memory configuration backends
//! (`git_config_backend_memory_options`, declared in `include/git2/sys/config.h`):
//! [`BackendMemoryOptions`] and its handles, which name the backend type and origin path
//! the backend's entries report.

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull};

use ffibox::{CBorrowedPtr, CCell, CMutSeam, CRefSeam};

use crate::ffi;

/// Caller-owned memory backend options, borrowing strings for `'o`.
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::config_mem::BackendMemoryOptions;
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
/// use libgit2::libgit2::src::libgit2::config_mem::BackendMemoryOptionsMut;
/// fn shrink<'a, 'o>(m: BackendMemoryOptionsMut<'a, 'static>)
///     -> BackendMemoryOptionsMut<'a, 'o> { m }
/// ```
///
/// Neither a handle nor a string view can outlive the options:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::config_mem::BackendMemoryOptions;
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
/// use libgit2::libgit2::src::libgit2::config_mem::BackendMemoryOptions;
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
/// use libgit2::libgit2::src::libgit2::config_mem::BackendMemoryOptions;
/// use std::ffi::CString;
/// let label = CString::new("temporary").unwrap();
/// let mut options = BackendMemoryOptions::new();
/// options.as_mut().set_backend_type(Some(&label));
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Wraps: `git_config_backend_memory_options`
/// Handwritten
#[repr(transparent)]
pub struct BackendMemoryOptions<'o>(
    pub(crate) ffi::git_config_backend_memory_options,
    pub(crate) PhantomData<(*const (), &'o CStr)>,
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
    type Mut<'a>
        = BackendMemoryOptionsMut<'a, 'o>
    where
        Self: 'a;
    type Ref<'a>
        = BackendMemoryOptionsRef<'a, 'o>
    where
        Self: 'a;
}

impl<'a, 'o: 'a> CRefSeam<'a> for BackendMemoryOptionsRef<'a, 'o> {
    type Cell = BackendMemoryOptions<'o>;
}

impl<'a, 'o: 'a> CMutSeam<'a> for BackendMemoryOptionsMut<'a, 'o> {
    type Cell = BackendMemoryOptions<'o>;
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
    /// Wraps: `git_config_backend_memory_options_init`
    ///
    /// # Panics
    ///
    /// Panics only if libgit2 refuses the version of the options it defines itself, which it never
    /// does.
    #[must_use]
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

    /// A shared handle over the same object, borrowed from this one.
    #[must_use]
    pub fn as_ref(&self) -> BackendMemoryOptionsRef<'_, 'o> {
        // SAFETY: initialized Rust-owned storage, shared for the handle's lifetime.
        BackendMemoryOptionsRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// An exclusive handle over the same object, reborrowed from this one.
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

impl<'a> BackendMemoryOptionsRef<'a, '_> {
    /// Structure version; constructors use the current supported version.
    ///
    /// Field: `git_config_backend_memory_options.version`
    #[must_use]
    pub fn version(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let version_ptr = unsafe { &raw const (*self.as_ptr()).version };
        // SAFETY: reads the initialized scalar through the live shared handle.
        unsafe { version_ptr.read() }
    }

    /// Unset means C will use "in-memory"; an empty string is distinct from unset.
    ///
    /// Field: `git_config_backend_memory_options.backend_type`
    #[must_use]
    pub fn backend_type(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let backend_type_ptr = unsafe { &raw const (*self.as_ptr()).backend_type };
        // SAFETY: reads the initialized pointer through the live shared handle.
        let value = unsafe { backend_type_ptr.read() };
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
    /// Field: `git_config_backend_memory_options.origin_path`
    #[must_use]
    pub fn origin_path(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let origin_path_ptr = unsafe { &raw const (*self.as_ptr()).origin_path };
        // SAFETY: reads the initialized pointer through the live shared handle.
        let value = unsafe { origin_path_ptr.read() };
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

impl<'o> BackendMemoryOptionsMut<'_, 'o> {
    /// A shared handle over the same object, borrowed from this one.
    #[must_use]
    pub fn as_ref(&self) -> BackendMemoryOptionsRef<'_, 'o> {
        self.0
    }

    /// An exclusive handle over the same object, reborrowed from this one.
    pub fn as_mut(&mut self) -> BackendMemoryOptionsMut<'_, 'o> {
        BackendMemoryOptionsMut(self.0, PhantomData)
    }

    /// Sets the scalar version. C's memory backend constructors currently ignore
    /// it; normally leave this at the constructor's supported version.
    pub fn set_version(&mut self, version: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let version_ptr = unsafe { &raw mut (*self.as_mut_ptr()).version };
        // SAFETY: exclusive live handle permits writing this initialized scalar.
        unsafe { version_ptr.write(version) };
    }

    /// Replaces the stored immutable borrow; never frees either string.
    pub fn set_backend_type(&mut self, value: Option<&'o CStr>) {
        let value = value.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let backend_type_ptr = unsafe { &raw mut (*self.as_mut_ptr()).backend_type };
        // SAFETY: exclusive live handle permits replacing the pointer; the new
        // referent is NUL-terminated and borrowed for the options' 'o.
        unsafe { backend_type_ptr.write(value) };
    }

    /// Replaces the stored immutable borrow; never frees either string.
    pub fn set_origin_path(&mut self, value: Option<&'o CStr>) {
        let value = value.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let origin_path_ptr = unsafe { &raw mut (*self.as_mut_ptr()).origin_path };
        // SAFETY: as set_backend_type, exclusive field access and borrow for 'o.
        unsafe { origin_path_ptr.write(value) };
    }
}
