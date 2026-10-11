//! Safe wrappers over `include/git2/sys/config.h`.
//!
//! Memory backend options are caller-owned values. Their nullable strings are
//! borrowed, never freed by C. `config_backend_new` copies both strings; neither
//! `git_config_backend_from_string` nor `git_config_backend_from_values` retains
//! the options. There is no C allocation, destruction or cloning primitive for
//! the options themselves. A Rust clone simply shares the immutable string borrows,
//! and the options cross threads as the borrows they hold do.

use core::ffi::CStr;
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CCell};

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
}
