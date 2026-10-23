//! Safe wrappers over `include/git2/config.h`.
//!
//! A [`Config`] is a `git_config`: an opaque, reference-counted set of configuration
//! backends (files, or custom backends) read in priority order and written to the
//! highest-priority writable one. libgit2 allocates it, counts its references
//! atomically, and releases one with `git_config_free`; the last release frees it,
//! unless a repository still owns it.
//!
//! Who holds the references decides the owner:
//!
//! - A [`ConfigBox`] is the only reference: what `git_config_new` (through [`Config::new`]) and the
//!   opening routines (`git_config_open_ondisk`, `git_config_open_default`,
//!   `git_config_open_level`, `git_config_open_global`) and snapshots (`git_config_snapshot`,
//!   `git_repository_config_snapshot`) hand out. It alone reaches the exclusive [`ConfigMut`].
//! - A [`ConfigArc`] is one of several references: what `git_repository_config` hands out, the
//!   repository holding another (and writing through it, as `git_repository_set_workdir` does), or
//!   a box once shared with [`From`](ConfigArc::from). It reaches only the shared [`ConfigRef`].
//!   libgit2 publishes no up-ref, so it does not clone.
//!
//! A sole reference stops being one when C takes another: a wrapper of
//! `git_repository_set_config`, which up-refs the config into the repository, must
//! take a [`ConfigArc`], not a [`ConfigBox`]'s handle.
//!
//! # The repository a config was loaded for
//!
//! A repository's config is loaded by `git_repository_config` (and the opening
//! routines that read it) through `git_config_add_file_ondisk(cfg, path, level, repo,
//! ..)`, and every file backend keeps that repository (`config_file_open`, `b->repo`)
//! for its `includeIf` conditions: each reparse of its file, when a `get` finds the
//! file changed (`config_file_refresh`) or after a write (`config_file_write`,
//! `config_file_refresh_from_buffer`), evaluates `gitdir:` through
//! `git_repository__item_path(repo)` and `onbranch:` through `git_repository_refdb(repo)`.
//! `git_repository_free` disowns the config (`GIT_REFCOUNT_OWN(config, NULL)`) but
//! clears none of those pointers, so a repository's config must not be used past the
//! repository: a [`Config<'r>`] borrows the repository it was loaded for, for `'r`,
//! which every owner and handle carries; a config loaded for no repository
//! (`git_config_new`, the opening routines given no repository, a snapshot, whose
//! backends copy entries and read nothing) is `Config<'static>`. The lifetime only
//! shrinks, as `&'r T`'s does.
//!
//! The wrappers of the routines that bind a config to a repository keep that
//! invariant: `git_repository_config` returns a `ConfigArc<'r>` of the repository's
//! borrow, and `git_repository_set_config`, which makes the repository use the config
//! for as long as it lives, installs only a `ConfigArc<'static>`, since the repository
//! would otherwise outlive the one the config's backends read.
//!
//! Level views also retain their original parent's backend, whose `cfg` pointer
//! may still address that parent. The returned owner's lifetime is therefore tied
//! to the parent even when no repository is involved. Shared handles open only
//! shared Rust views; exclusive handles can open writable views while freezing
//! the parent, preventing recursive backend locking through a sibling view.
//!
//! # Threads
//!
//! The struct is opaque (its body is in `src/libgit2/config.h`), so there are no field
//! accessors: the handles carry the routines of `git2/config.h`, which their own runs
//! add. `Config` asserts neither `Send` nor `Sync`: libgit2 documents only snapshots
//! as shareable across threads, nothing synchronises the backend lists a config
//! mutates, and a repository-owned config reads its owner (`GIT_REFCOUNT_OWNER`) in
//! `git_config_set_string` to clear the owner's cache, racing a release of that
//! repository on another thread.

use core::ffi::{CStr, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::num::NonZero;
use core::ops::ControlFlow;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use std::any::Any;
use std::ffi::CString;
use std::panic::{self, AssertUnwindSafe};

use ffibox::{CArc, CBorrowedPtr, CBox, CCell, CDrop, CNew, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::config::sys::config::{BackendMut, IteratorBox, IteratorMut};
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::{clear, runtime_pinned, set};

/// A configuration (`git_config`): an opaque, reference-counted object of
/// libgit2's, reached only behind a pointer.
///
/// Owned as a [`ConfigBox`] when the reference is the only one, which
/// [`new`](Self::new) builds, or as a [`ConfigArc`] when others exist; see the
/// [module docs](self). Neither crosses threads. `'r` is the borrow of the
/// repository the config was loaded for (see the
/// [module docs](self#the-repository-a-config-was-loaded-for)); a config loaded for
/// none is `Config<'static>`, and the lifetime only shrinks:
///
/// ```compile_fail
/// use libgit2::libgit2::config::config::ConfigArc;
///
/// fn widen<'r>(config: ConfigArc<'r>) -> ConfigArc<'static> {
///     config
/// }
/// ```
///
/// while the exclusive handle is invariant in it, as `&mut` is in its pointee:
///
/// ```compile_fail
/// use libgit2::libgit2::config::config::ConfigMut;
///
/// fn shrink<'a, 'r>(m: ConfigMut<'a, 'static>) -> ConfigMut<'a, 'r> {
///     m
/// }
/// ```
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::config::config::{Config, ConfigArc};
///
/// let shared = ConfigArc::from(Config::new().unwrap());
/// let _ = shared.clone();
/// ```
///
/// A shared reference has no exclusive handle, since others may use the config:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::config::config::{Config, ConfigArc};
///
/// let mut shared = ConfigArc::from(Config::new().unwrap());
/// let _ = shared.as_mut();
/// ```
///
/// Neither owner crosses threads:
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::config::config::Config;
///
/// let sole = Config::new().unwrap();
/// std::thread::spawn(move || drop(sole));
/// ```
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::config::config::{Config, ConfigArc};
///
/// let shared = ConfigArc::from(Config::new().unwrap());
/// std::thread::spawn(move || drop(shared));
/// ```
///
/// and no handle outlives its owner:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::config::config::Config;
///
/// let config = Config::new().unwrap();
/// let handle = config.as_ref();
/// drop(config);
/// let _ = handle;
/// ```
///
/// Wraps: `git_config`
/// Handwritten
#[repr(transparent)]
pub struct Config<'r>(
    ffi::git_config,
    PhantomData<(*const (), &'r ffi::git_repository)>,
);

/// Shared borrow of a [`Config`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ConfigRef<'a, 'r>(CBorrowedPtr<'a, Config<'r>>);

/// Exclusive borrow of a [`Config`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct ConfigMut<'a, 'r>(
    ConfigRef<'a, 'r>,
    // Invariant in the config, and so in `'r`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut Config<'r>>,
);

// SAFETY: `Config` is `#[repr(transparent)]` over `git_config` with a zero-sized
// marker; both handles are `#[repr(transparent)]` over `CBorrowedPtr<'a, Config<'r>>`
// (`ConfigMut` through `ConfigRef`) with zero-sized markers and no `Drop`, hand out no
// reference to the config, and `ConfigRef` has no write operation. `ConfigMut` is
// invariant in the config through its `PhantomData<&'a mut Config<'r>>`.
unsafe impl<'r> CCell for Config<'r> {
    type C = ffi::git_config;
    type Mut<'a>
        = ConfigMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = ConfigRef<'a, 'r>
    where
        Self: 'a;
}

impl<'r> ConfigRef<'_, 'r> {
    /// Borrows the config at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must be NULL or address a live config that stays live for `'a`, loaded
    /// for no repository or for one that outlives `'r`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_config) -> Option<Self> {
        // SAFETY: this function's contract is `CCell::ref_from_ptr`'s: `ptr` is NULL or a
        // live `git_config` that outlives `'a` and is not written meanwhile.
        unsafe { <Config<'r> as CCell>::ref_from_ptr(ptr) }
    }

    /// Pointer to the C object, for the routines that take a `git_config *` and
    /// only read it.
    pub(crate) fn as_ptr(&self) -> *mut ffi::git_config {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> ConfigMut<'_, 'r> {
    /// Writable pointer to the C object.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_config {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> ConfigRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> ConfigMut<'_, 'r> {
        ConfigMut(self.0, PhantomData)
    }
}

impl ConfigRef<'_, '_> {
    /// Copies the current entries into an independent, read-only configuration.
    /// Neither this config nor its repository must outlive the result. Later
    /// changes to the source do not change the snapshot. The result uses the
    /// existing config release policy and remains thread-bound, like `Config`.
    ///
    /// Wraps: `git_config_snapshot`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when the snapshot cannot be made: a backend that cannot
    /// reread its changed file, or an allocation failure while copying the entries.
    pub fn snapshot(&self) -> Result<ConfigBox<'static>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: this handle keeps the source and its repository live. `out` is
        // a writable pointer slot. C copies entries synchronously, retaining no
        // Rust borrow, and frees partial construction on failure.
        let rc = unsafe { ffi::git_config_snapshot(&raw mut out, self.as_ptr()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: success returns a fresh sole reference paired with
        // git_config_free. Its backends own copied entries and never access the
        // source/repository after construction, so the owner is `'static`.
        unsafe { ConfigBox::from_c(out) }.ok_or_else(|| ConfigError::from_libgit2(-1))
    }
}

impl ConfigMut<'_, '_> {
    /// Sets a string in the first enabled, writable backend. Both strings are
    /// borrowed only for this call; C copies the value. Empty and non-UTF-8
    /// values are supported. Invalid names, non-unique keys, locked files and
    /// configurations without a writer return C's code and copied error.
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::Config;
    /// let config = Config::new().unwrap();
    /// config.as_ref().set_string(c"a.b", c"value");
    /// ```
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::Config;
    /// let mut config = Config::new().unwrap();
    /// let shared = config.as_ref();
    /// config.as_mut().set_string(c"a.b", c"value");
    /// let _ = shared.snapshot();
    /// ```
    ///
    /// Wraps: `git_config_set_string`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and copied error for an invalid name, a key that names several
    /// values, a locked file, or a configuration without a writable backend.
    pub fn set_string(&mut self, name: &CStr, value: &CStr) -> Result<(), ConfigError> {
        let config = self.as_mut_ptr();
        // SAFETY: the exclusive handle permits mutation and keeps any repository
        // the backend reads live. Both strings are nonnull and NUL-terminated;
        // C reads/copies them during this call and retains neither pointer.
        let rc = unsafe { ffi::git_config_set_string(config, name.as_ptr(), value.as_ptr()) };
        config_status(rc)
    }

    /// Enables writing only to the listed levels; an empty slice disables all
    /// writers. Missing levels are ignored and duplicate levels use their first
    /// index. C reads but does not retain or modify the slice.
    ///
    /// C assigns higher write priority to later indices. However, its cached
    /// sorted-vector flag currently prevents re-sorting existing backends, so
    /// their original order is retained. This wrapper preserves that behavior;
    /// selecting a single level reliably selects that backend. This does not
    /// change read priority or make read-only backends writable.
    ///
    /// Wraps: `git_config_set_writeorder`
    ///
    /// # Errors
    ///
    /// Fails before calling C if the length is not less than `i32::MAX`, which
    /// C requires to represent indices and safely compare priorities.
    pub fn set_write_order(&mut self, levels: &[ConfigLevel]) -> Result<(), ConfigError> {
        validate_write_order_len(levels.len())?;
        // SAFETY: exclusive config access, and a readable contiguous slice of
        // transparent git_config_level_t wrappers. Despite its mutable C type,
        // C only reads levels[0..len] and retains no pointer. For zero length C
        // never dereferences levels; NULL represents that case explicitly.
        let rc = unsafe {
            let levels_ptr = if levels.is_empty() {
                ptr::null_mut()
            } else {
                levels.as_ptr().cast::<ffi::git_config_level_t>().cast_mut()
            };
            ffi::git_config_set_writeorder(self.as_mut_ptr(), levels_ptr, levels.len())
        };
        config_status(rc)
    }
}

fn config_status(rc: c_int) -> Result<(), ConfigError> {
    if rc < 0 {
        Err(ConfigError::from_libgit2(rc))
    } else {
        Ok(())
    }
}

/// The status of a delete of `name`, called with the thread's error cleared.
///
/// `git_config_delete_entry` and `git_config_delete_multivar` return `GIT_EREADONLY`
/// without setting an error when the config has no writer, so `GIT_ERROR_NONE` after
/// that code is libgit2's silence: report it as `git_config_set_string` reports the
/// same condition.
fn delete_status(rc: c_int, name: &CStr) -> Result<(), ConfigError> {
    if rc == ffi::GIT_EREADONLY && last().class() == Some(ErrorClass::None) {
        set(ErrorClass::Config, &readonly_delete_error(name));
    }
    config_status(rc)
}

/// The error a delete of `name` reports for a config without a writer.
fn readonly_delete_error(name: &CStr) -> CString {
    let message = [
        b"cannot delete '".as_slice(),
        name.to_bytes(),
        b"': the configuration is read-only",
    ]
    .concat();
    CString::new(message).expect("a C string's bytes and the message hold no NUL")
}

fn validate_write_order_len(len: usize) -> Result<(), ConfigError> {
    if len >= c_int::MAX as usize {
        set(
            ErrorClass::Invalid,
            c"write order length must be less than INT_MAX",
        );
        return Err(ConfigError::from_libgit2(-1));
    }
    Ok(())
}

impl fmt::Debug for ConfigRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ConfigRef").field(&self.as_ptr()).finish()
    }
}

impl fmt::Debug for ConfigMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases one reference to a config through `git_config_free`, which frees the
/// config, its backend list and every backend it alone holds once the count reaches
/// zero and no repository owns it; builds one through `git_config_new`.
///
/// Wraps: `git_config_free`
/// Wraps: `git_config_new`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct ConfigFree;

// SAFETY: `git_config_free` drops exactly the one reference the owner holds: the config
// is freed once, when its count reaches zero and no repository owns it. It needs no
// state beyond the pointer, and reaches no repository.
unsafe impl<'r> CDrop<Config<'r>> for ConfigFree {
    unsafe fn c_drop(&self, ptr: NonNull<Config<'r>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live config whose
        // reference the caller gives up; `Config` is transparent over `git_config`.
        unsafe { ffi::git_config_free(ptr.as_ptr().cast()) }
    }
}

// SAFETY: `git_config_new` hands out a fresh `git__calloc`'d config with empty backend
// lists and a reference count of 1, owned by and loaded for no repository (so
// `'static`): one reference, which `git_config_free` releases, freeing it. It writes
// `out` only on success, so a failure (out of memory, or libgit2 not initialisable) is
// `None`.
unsafe impl CNew<Config<'static>> for ConfigFree {
    fn c_new(&self) -> Option<NonNull<Config<'static>>> {
        if !runtime_pinned() {
            return None;
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot. The runtime is pinned, so the allocation
        // goes through an initialised allocator and a failure is reported on live
        // thread-local state.
        let rc = unsafe { ffi::git_config_new(&raw mut out) };
        if rc < 0 {
            return None;
        }
        NonNull::new(out.cast())
    }
}

/// The only reference to a [`Config`], released by `git_config_free` on drop.
///
/// `'static` means the config was loaded for no repository; a `ConfigBox<'r>`
/// borrows the repository its backends read, so it cannot outlive it.
pub type ConfigBox<'r> = CBox<Config<'r>, ConfigFree>;

/// One of several references to a [`Config`], released by `git_config_free` on drop;
/// not `Clone`, since libgit2 publishes no up-ref.
///
/// `'static` means the config was loaded for no repository (see the
/// [module docs](self#the-repository-a-config-was-loaded-for)); `ConfigArc<'r>`,
/// which the wrapper of `git_repository_config` returns, borrows that repository, so
/// neither it nor a handle it lends outlives the repository:
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::repository::repository::Repository;
///
/// let repo = Repository::open(c".").unwrap();
/// let config = repo.as_ref().config().unwrap();
/// drop(repo);
/// drop(config);
/// ```
pub type ConfigArc<'r> = CArc<Config<'r>, ConfigFree>;

impl Config<'_> {
    /// An empty configuration, with no backends: every lookup misses, and every
    /// write fails until a backend is added.
    ///
    /// Wraps: `git_config_new`
    ///
    /// # Errors
    ///
    /// Pins the runtime first. Fails, with libgit2's error, when the config cannot be
    /// allocated or libgit2 cannot be initialised.
    pub fn new() -> Result<ConfigBox<'static>, ErrorBuf> {
        ConfigBox::new().ok_or_else(last)
    }
}

// A `Config` is only ever behind a pointer, so the owners are pointer-sized, and an
// optional one is a nullable `git_config *`.
const _: () = assert!(
    size_of::<ConfigBox<'static>>() == size_of::<*mut ffi::git_config>()
        && size_of::<ConfigArc<'static>>() == size_of::<*mut ffi::git_config>()
        && size_of::<Option<ConfigBox<'static>>>() == size_of::<*mut ffi::git_config>()
);

// Config constructors and accessors keep foreign ownership inside ffibox owners.
impl Config<'_> {
    /// Opens the file at `path` as the only backend, at [`ConfigLevel::LOCAL`]. Missing
    /// files are accepted and created by a later write. The path is copied; no
    /// repository is retained.
    ///
    /// Wraps: `git_config_open_ondisk`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when libgit2 cannot be initialised, or when `path`
    /// exists but cannot be read or parsed.
    pub fn open(path: &CStr) -> Result<ConfigBox<'static>, ConfigError> {
        if !runtime_pinned() {
            return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
        }
        let mut out = ptr::null_mut();
        // SAFETY: one writable output slot and a live NUL-terminated path, copied
        // by C. Success transfers a new config; failure frees partial construction.
        let rc = unsafe { ffi::git_config_open_ondisk(&raw mut out, path.as_ptr()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: C transfers one fresh config reference on success. The result
        // lifetime is no longer than any retained parent and repository borrow.
        unsafe { ConfigBox::from_c(out) }.ok_or_else(|| ConfigError::from_libgit2(ffi::GIT_ERROR))
    }

    /// Opens the default global, XDG and system configuration locations.
    ///
    /// Wraps: `git_config_open_default`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when libgit2 cannot be initialised, or when one of the
    /// default files exists but cannot be read or parsed.
    pub fn open_default() -> Result<ConfigBox<'static>, ConfigError> {
        if !runtime_pinned() {
            return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
        }
        let mut out = ptr::null_mut();
        // SAFETY: C initializes this writable slot with an independent config
        // loaded for no repository, or frees it on failure. Runtime is pinned.
        let rc = unsafe { ffi::git_config_open_default(&raw mut out) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: C transfers one fresh config reference on success. The result
        // lifetime is no longer than any retained parent and repository borrow.
        unsafe { ConfigBox::from_c(out) }.ok_or_else(|| ConfigError::from_libgit2(ffi::GIT_ERROR))
    }
}

impl<'a, 'r: 'a> ConfigRef<'a, 'r> {
    /// Opens a read-only Rust view of one priority. C shares the backend, not the
    /// config allocation. HIGHEST produces a view with no enabled writer.
    /// The view keeps the parent borrowed (custom backends may
    /// use their parent pointer). Use the exclusive handle to obtain a writable view.
    ///
    /// Wraps: `git_config_open_level`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend has that priority, and with libgit2's
    /// code and error when the view cannot be allocated.
    pub fn open_level(&self, level: ConfigLevel) -> Result<ConfigArc<'a>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: parent is live for 'a and C only reads its backend vector. C
        // uprefs the selected backend into a fresh config, tied here to the parent.
        let rc = unsafe { ffi::git_config_open_level(&raw mut out, self.as_ptr(), level.raw()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: successful output owns one fresh config reference; the shared
        // view keeps its parent (and repository) borrowed for the result lifetime.
        unsafe { ConfigBox::from_c(out) }
            .ok_or_else(|| ConfigError::from_libgit2(ffi::GIT_ERROR))
            .map(ConfigArc::from)
    }

    /// Opens XDG when present, otherwise global, as a shared Rust view.
    ///
    /// Wraps: `git_config_open_global`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when neither an XDG nor a global backend is present, and
    /// with libgit2's code and error when the view cannot be allocated.
    pub fn open_global(&self) -> Result<ConfigArc<'a>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: live parent borrowed for 'a; C returns a fresh config holding
        // one retained backend. The returned shared owner keeps the parent borrowed.
        let rc = unsafe { ffi::git_config_open_global(&raw mut out, self.as_ptr()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: successful output owns one fresh config reference; the shared
        // view keeps its parent (and repository) borrowed for the result lifetime.
        unsafe { ConfigBox::from_c(out) }
            .ok_or_else(|| ConfigError::from_libgit2(ffi::GIT_ERROR))
            .map(ConfigArc::from)
    }

    /// Reads and parses a configuration bool value, returning an owned scalar.
    ///
    /// Wraps: `git_config_get_bool`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend holds `name`, and with libgit2's code and
    /// error for an invalid name or a value that does not parse as a boolean.
    pub fn get_bool(&self, name: &CStr) -> Result<bool, ConfigError> {
        let mut out = 0;
        // SAFETY: config is live, name is NUL-terminated, and out is a writable
        // aligned scalar. C temporarily retains and releases its entry; no borrow
        // escapes this call, even when a live backend refreshes its file.
        let rc = unsafe { ffi::git_config_get_bool(&raw mut out, self.as_ptr(), name.as_ptr()) };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(out != 0)
        }
    }

    /// Reads and parses a configuration i32 value, returning an owned scalar.
    ///
    /// Wraps: `git_config_get_int32`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend holds `name`, and with libgit2's code and
    /// error for an invalid name or a value that does not parse as a 32-bit integer or overflows
    /// it.
    pub fn get_i32(&self, name: &CStr) -> Result<i32, ConfigError> {
        let mut out = 0;
        // SAFETY: config is live, name is NUL-terminated, and out is a writable
        // aligned scalar. C temporarily retains and releases its entry; no borrow
        // escapes this call, even when a live backend refreshes its file.
        let rc = unsafe { ffi::git_config_get_int32(&raw mut out, self.as_ptr(), name.as_ptr()) };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(out)
        }
    }

    /// Reads and parses a configuration i64 value, returning an owned scalar.
    /// Clears stale errors first and reports C's silent suffix-tail parse failure
    /// as a configuration parse error instead of attributing a previous failure.
    ///
    /// Wraps: `git_config_get_int64`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend holds `name`, and with libgit2's code and
    /// error for an invalid name or a value that does not parse as a 64-bit integer.
    pub fn get_i64(&self, name: &CStr) -> Result<i64, ConfigError> {
        let mut out = 0;
        clear();
        // SAFETY: config is live, name is NUL-terminated, and out is a writable
        // aligned scalar. C temporarily retains and releases its entry; no borrow
        // escapes this call, even when a live backend refreshes its file.
        let rc = unsafe { ffi::git_config_get_int64(&raw mut out, self.as_ptr(), name.as_ptr()) };
        if rc < 0 {
            if last().class() == Some(ErrorClass::None) {
                set(
                    ErrorClass::Config,
                    c"failed to parse configuration value as an integer",
                );
            }
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(out)
        }
    }

    /// Reads C's snapshot-only string accessor and copies its borrowed result.
    /// A live writable config is rejected by C. A valueless entry yields empty.
    /// Returning an owned string prevents backend sharing or future calls from
    /// invalidating a Rust string reference.
    ///
    /// Wraps: `git_config_get_string`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend holds `name`, and with libgit2's code and
    /// error for an invalid name or a live configuration, which C refuses for this accessor.
    pub fn get_string(&self, name: &CStr) -> Result<CString, ConfigError> {
        let mut out = ptr::null();
        // SAFETY: live config and name, writable pointer slot. On success C's
        // read-only backend keeps the NUL-terminated result live through this copy.
        let rc = unsafe { ffi::git_config_get_string(&raw mut out, self.as_ptr(), name.as_ptr()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: successful C get_string returns a non-NULL NUL-terminated string;
        // no foreign call or write intervenes before the owned copy is complete.
        Ok(unsafe { CStr::from_ptr(out) }.to_owned())
    }

    /// Copies a value from either a live config or snapshot into an owned buffer.
    /// Valueless entries are empty strings, distinct from missing entries.
    ///
    /// Wraps: `git_config_get_string_buf`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend holds `name`, and with libgit2's code and
    /// error for an invalid name or an allocation failure.
    pub fn get_string_buf(
        &self,
        name: &CStr,
    ) -> Result<crate::libgit2::core::buffer::BufVal, ConfigError> {
        let mut out = crate::libgit2::core::buffer::Buf::new();
        // SAFETY: live config and NUL-terminated name; exclusive valid git_buf
        // storage. C transfers its allocation to out, which disposes it on all paths.
        let rc = unsafe {
            ffi::git_config_get_string_buf(out.as_mut().as_mut_ptr(), self.as_ptr(), name.as_ptr())
        };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(out)
        }
    }

    /// Copies a path and expands a leading `~/` (or `~`) using C's home lookup.
    /// Named users (`~user`) and valueless path entries are rejected by C.
    ///
    /// Wraps: `git_config_get_path`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend holds `name`, and with libgit2's code and
    /// error for a `~user` path or a valueless entry, which C refuses, or when the home directory
    /// cannot be found.
    pub fn get_path(
        &self,
        name: &CStr,
    ) -> Result<crate::libgit2::core::buffer::BufVal, ConfigError> {
        let mut out = crate::libgit2::core::buffer::Buf::new();
        // SAFETY: config and name are live; out is exclusively borrowed initialized
        // buffer storage. C owns entry retention and releases it, and transfers any
        // output allocation into the RAII buffer, including on error.
        let rc = unsafe {
            ffi::git_config_get_path(out.as_mut().as_mut_ptr(), self.as_ptr(), name.as_ptr())
        };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(out)
        }
    }
}

impl<'r> ConfigMut<'_, 'r> {
    /// Adds a copied file path at a priority, optionally replacing that priority.
    /// A repository is retained as a borrow for includeIf evaluation on every reparse.
    /// It must outlive the config's `'r`; narrow a fresh owner's lifetime before
    /// obtaining its mutable handle to attach a non-static repository.
    ///
    /// Wraps: `git_config_add_file_ondisk`
    ///
    /// # Errors
    ///
    /// Registration accepts `0..i32::MAX` (exclusive). Negative levels are selectors,
    /// not priorities. Excluding MAX also prevents C comparator subtraction overflow
    /// in a view opened with HIGHEST, which C registers internally at -1.
    ///
    /// Fails with [`ErrorCode::Invalid`] for a level outside `0..i32::MAX`, before calling C; with
    /// [`ErrorCode::Exists`] when a backend already has that priority and `force` is false; and
    /// with libgit2's code and error when the file exists but cannot be read or parsed.
    pub fn add_file(
        &mut self,
        path: &CStr,
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
        // SAFETY: exclusive live config, copied NUL-terminated path, safe priority.
        // Repository is NULL or kept live for the config's 'r; C stores only that
        // borrow and owns the newly constructed backend on success, frees on failure.
        let rc = unsafe {
            ffi::git_config_add_file_ondisk(
                self.as_mut_ptr(),
                path.as_ptr(),
                level.raw(),
                repository.map_or(ptr::null(), |r| r.as_ptr()),
                force.into(),
            )
        };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }

    /// Opens a level view, freezing this parent handle while it lives.
    /// HIGHEST selects the top backend but C disables its writer; concrete levels
    /// remain writable when their backend is writable.
    /// C retains the backend; the exclusive borrow prevents two views from locking
    /// the same backend recursively or replacing it through the parent.
    ///
    /// Wraps: `git_config_open_level`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend has that priority, and with libgit2's
    /// code and error when the view cannot be allocated.
    pub fn open_level(&mut self, level: ConfigLevel) -> Result<ConfigBox<'_>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: parent is live and exclusively borrowed for the returned view.
        // Success constructs a fresh config with one retained backend instance.
        let rc =
            unsafe { ffi::git_config_open_level(&raw mut out, self.as_mut_ptr(), level.raw()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: C transfers one fresh config reference on success. The result
        // lifetime is no longer than any retained parent and repository borrow.
        unsafe { ConfigBox::from_c(out) }.ok_or_else(|| ConfigError::from_libgit2(ffi::GIT_ERROR))
    }

    /// Opens a writable XDG view, falling back to global. Keeps the parent exclusive.
    ///
    /// Wraps: `git_config_open_global`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when neither an XDG nor a global backend is present, and
    /// with libgit2's code and error when the view cannot be allocated.
    pub fn open_global(&mut self) -> Result<ConfigBox<'_>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: as open_level: fresh config over a retained backend; the returned
        // owner's lifetime freezes the parent until the view is released.
        let rc = unsafe { ffi::git_config_open_global(&raw mut out, self.as_mut_ptr()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: C transfers one fresh config reference on success. The result
        // lifetime is no longer than any retained parent and repository borrow.
        unsafe { ConfigBox::from_c(out) }.ok_or_else(|| ConfigError::from_libgit2(ffi::GIT_ERROR))
    }

    /// Writes an owned scalar to the highest-priority writable backend.
    ///
    /// Wraps: `git_config_set_bool`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid name, a key that names several values, a
    /// locked file, or a configuration without a writable backend.
    pub fn set_bool(&mut self, name: &CStr, value: bool) -> Result<(), ConfigError> {
        let config = self.as_mut_ptr();
        // SAFETY: exclusive live config and a NUL-terminated name borrowed only
        // for this call. C formats and copies the scalar, retaining neither input.
        let rc = unsafe { ffi::git_config_set_bool(config, name.as_ptr(), value.into()) };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }

    /// Writes an owned scalar to the highest-priority writable backend.
    ///
    /// Wraps: `git_config_set_int32`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid name, a key that names several values, a
    /// locked file, or a configuration without a writable backend.
    pub fn set_i32(&mut self, name: &CStr, value: i32) -> Result<(), ConfigError> {
        // SAFETY: exclusive live config and a NUL-terminated name borrowed only
        // for this call. C formats and copies the scalar, retaining neither input.
        let rc = unsafe { ffi::git_config_set_int32(self.as_mut_ptr(), name.as_ptr(), value) };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }

    /// Writes an owned scalar to the highest-priority writable backend.
    ///
    /// Wraps: `git_config_set_int64`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid name, a key that names several values, a
    /// locked file, or a configuration without a writable backend.
    pub fn set_i64(&mut self, name: &CStr, value: i64) -> Result<(), ConfigError> {
        // SAFETY: exclusive live config and a NUL-terminated name borrowed only
        // for this call. C formats and copies the scalar, retaining neither input.
        let rc = unsafe { ffi::git_config_set_int64(self.as_mut_ptr(), name.as_ptr(), value) };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }

    /// Deletes a single-valued entry from the writer backend. The file backend
    /// refuses a multivar (use [`delete_multivar`](Self::delete_multivar)) and a
    /// missing key. The name is borrowed only for this call.
    ///
    /// Wraps: `git_config_delete_entry`
    ///
    /// # Errors
    ///
    /// Without a writable backend this fails with [`ErrorCode::ReadOnly`] and a
    /// configuration error saying so. libgit2 sets no error on that path, leaving
    /// whatever an earlier call set, so the wrapper clears the thread's error before
    /// the call and sets its own; a success therefore leaves no earlier error behind.
    ///
    /// Fails, as well, with [`ErrorCode::NotFound`] for a missing key, and with libgit2's code and
    /// error for a key that names several values.
    pub fn delete_entry(&mut self, name: &CStr) -> Result<(), ConfigError> {
        clear();
        // SAFETY: exclusive live config, live NUL-terminated name, no retained Rust
        // pointer: the backend normalizes and copies the name during the call.
        let rc = unsafe { ffi::git_config_delete_entry(self.as_mut_ptr(), name.as_ptr()) };
        delete_status(rc, name)
    }

    /// Deletes the values of a multivar that match `regexp` from the writer backend.
    /// Both strings are borrowed only for this call; the backend compiles and frees
    /// its own regex.
    ///
    /// Wraps: `git_config_delete_multivar`
    ///
    /// # Errors
    ///
    /// Without a writable backend this fails as [`delete_entry`](Self::delete_entry)
    /// does, with an error of its own.
    ///
    /// Fails with libgit2's code and error for an invalid name or pattern, and
    /// [`ErrorCode::NotFound`] when no value matches.
    pub fn delete_multivar(&mut self, name: &CStr, regexp: &CStr) -> Result<(), ConfigError> {
        clear();
        // SAFETY: exclusive live config, live NUL-terminated inputs, no retained
        // Rust pointers. Backend code owns regex compilation, copying and cleanup.
        let rc = unsafe {
            ffi::git_config_delete_multivar(self.as_mut_ptr(), name.as_ptr(), regexp.as_ptr())
        };
        delete_status(rc, name)
    }

    /// Replaces the values of a multivar that match `regexp`, or appends `value` when
    /// none match, in the writer backend. Strings are borrowed only for this call;
    /// the value is copied.
    ///
    /// Wraps: `git_config_set_multivar`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid name or pattern, a locked file, or a
    /// configuration without a writable backend.
    pub fn set_multivar(
        &mut self,
        name: &CStr,
        regexp: &CStr,
        value: &CStr,
    ) -> Result<(), ConfigError> {
        // SAFETY: exclusive live config, live NUL-terminated inputs, no retained
        // Rust pointers. Backend code owns regex compilation, copying and cleanup.
        let rc = unsafe {
            ffi::git_config_set_multivar(
                self.as_mut_ptr(),
                name.as_ptr(),
                regexp.as_ptr(),
                value.as_ptr(),
            )
        };
        if rc < 0 {
            Err(ConfigError::from_libgit2(rc))
        } else {
            Ok(())
        }
    }
}

// Transaction mutations are distinct safe variants: the transaction already owns
// the exclusive config borrow and must not expose it for recursive locking.
impl crate::libgit2::refs::transaction::ConfigTransaction<'_, '_> {
    /// Mutates the locked writer through the transaction's retained exclusive
    /// config borrow, without allowing another lock of that same backend.
    ///
    /// Wraps: `git_config_set_string`
    ///
    /// # Errors
    ///
    /// Fails as `ConfigMut::set_string` does.
    pub fn set_string(&mut self, name: &CStr, value: &CStr) -> Result<(), ConfigError> {
        self.config_mut().set_string(name, value)
    }

    /// Mutates the locked writer through the transaction's retained exclusive
    /// config borrow, without allowing another lock of that same backend.
    ///
    /// Wraps: `git_config_set_bool`
    ///
    /// # Errors
    ///
    /// Fails as `ConfigMut::set_bool` does.
    pub fn set_bool(&mut self, name: &CStr, value: bool) -> Result<(), ConfigError> {
        self.config_mut().set_bool(name, value)
    }

    /// Mutates the locked writer through the transaction's retained exclusive
    /// config borrow, without allowing another lock of that same backend.
    ///
    /// Wraps: `git_config_set_int32`
    ///
    /// # Errors
    ///
    /// Fails as `ConfigMut::set_i32` does.
    pub fn set_i32(&mut self, name: &CStr, value: i32) -> Result<(), ConfigError> {
        self.config_mut().set_i32(name, value)
    }

    /// Mutates the locked writer through the transaction's retained exclusive
    /// config borrow, without allowing another lock of that same backend.
    ///
    /// Wraps: `git_config_set_int64`
    ///
    /// # Errors
    ///
    /// Fails as `ConfigMut::set_i64` does.
    pub fn set_i64(&mut self, name: &CStr, value: i64) -> Result<(), ConfigError> {
        self.config_mut().set_i64(name, value)
    }

    /// Mutates the locked writer through the transaction's retained exclusive
    /// config borrow, without allowing another lock of that same backend.
    ///
    /// Wraps: `git_config_set_multivar`
    ///
    /// # Errors
    ///
    /// Fails as `ConfigMut::set_multivar` does.
    pub fn set_multivar(
        &mut self,
        name: &CStr,
        regexp: &CStr,
        value: &CStr,
    ) -> Result<(), ConfigError> {
        self.config_mut().set_multivar(name, regexp, value)
    }

    /// Mutates the locked writer through the transaction's retained exclusive
    /// config borrow, without allowing another lock of that same backend.
    ///
    /// Wraps: `git_config_delete_entry`
    ///
    /// # Errors
    ///
    /// Fails as `ConfigMut::delete_entry` does.
    pub fn delete_entry(&mut self, name: &CStr) -> Result<(), ConfigError> {
        self.config_mut().delete_entry(name)
    }

    /// Mutates the locked writer through the transaction's retained exclusive
    /// config borrow, without allowing another lock of that same backend.
    ///
    /// Wraps: `git_config_delete_multivar`
    ///
    /// # Errors
    ///
    /// Fails as `ConfigMut::delete_multivar` does.
    pub fn delete_multivar(&mut self, name: &CStr, regexp: &CStr) -> Result<(), ConfigError> {
        self.config_mut().delete_multivar(name, regexp)
    }
}

/// Finds the existing global config file using C's search paths. `NotFound` is an
/// error with its original code; no UTF-8 assumption is made about the output.
///
/// Wraps: `git_config_find_global`
///
/// # Errors
///
/// Fails with [`ErrorCode::NotFound`] when no global configuration file exists, and with libgit2's
/// code and error when libgit2 cannot be initialised or the search paths cannot be read.
pub fn find_global() -> Result<crate::libgit2::core::buffer::BufVal, ConfigError> {
    if !runtime_pinned() {
        return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
    }
    let mut out = crate::libgit2::core::buffer::Buf::new();
    // SAFETY: runtime pinned; exclusive initialized git_buf. C transfers any
    // allocation to the buffer, whose disposer handles success and failure alike.
    let rc = unsafe { ffi::git_config_find_global(out.as_mut().as_mut_ptr()) };
    if rc < 0 {
        Err(ConfigError::from_libgit2(rc))
    } else {
        Ok(out)
    }
}

/// Finds the existing system config file using C's search paths. `NotFound` is an
/// error with its original code; no UTF-8 assumption is made about the output.
///
/// Wraps: `git_config_find_system`
///
/// # Errors
///
/// Fails with [`ErrorCode::NotFound`] when no system configuration file exists, and with libgit2's
/// code and error when libgit2 cannot be initialised or the search paths cannot be read.
pub fn find_system() -> Result<crate::libgit2::core::buffer::BufVal, ConfigError> {
    if !runtime_pinned() {
        return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
    }
    let mut out = crate::libgit2::core::buffer::Buf::new();
    // SAFETY: runtime pinned; exclusive initialized git_buf. C transfers any
    // allocation to the buffer, whose disposer handles success and failure alike.
    let rc = unsafe { ffi::git_config_find_system(out.as_mut().as_mut_ptr()) };
    if rc < 0 {
        Err(ConfigError::from_libgit2(rc))
    } else {
        Ok(out)
    }
}

/// Finds the existing xdg config file using C's search paths. `NotFound` is an
/// error with its original code; no UTF-8 assumption is made about the output.
///
/// Wraps: `git_config_find_xdg`
///
/// # Errors
///
/// Fails with [`ErrorCode::NotFound`] when no XDG configuration file exists, and with libgit2's
/// code and error when libgit2 cannot be initialised or the search paths cannot be read.
pub fn find_xdg() -> Result<crate::libgit2::core::buffer::BufVal, ConfigError> {
    if !runtime_pinned() {
        return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
    }
    let mut out = crate::libgit2::core::buffer::Buf::new();
    // SAFETY: runtime pinned; exclusive initialized git_buf. C transfers any
    // allocation to the buffer, whose disposer handles success and failure alike.
    let rc = unsafe { ffi::git_config_find_xdg(out.as_mut().as_mut_ptr()) };
    if rc < 0 {
        Err(ConfigError::from_libgit2(rc))
    } else {
        Ok(out)
    }
}

/// Expands `~` or `~/...` and copies other paths verbatim. Named users are unsupported.
/// Input is borrowed only for the call; the returned buffer owns its bytes.
///
/// Wraps: `git_config_parse_path`
///
/// # Errors
///
/// Fails with libgit2's code and error for a `~user` path, when the home directory cannot be found,
/// or when libgit2 cannot be initialised.
pub fn parse_path(value: &CStr) -> Result<crate::libgit2::core::buffer::BufVal, ConfigError> {
    if !runtime_pinned() {
        return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
    }
    let mut out = crate::libgit2::core::buffer::Buf::new();
    // SAFETY: a NUL-terminated live input and exclusively borrowed initialized
    // buffer; C retains no input, transfers output to the buffer's disposer.
    let rc = unsafe { ffi::git_config_parse_path(out.as_mut().as_mut_ptr(), value.as_ptr()) };
    if rc < 0 {
        Err(ConfigError::from_libgit2(rc))
    } else {
        Ok(out)
    }
}

/// Parses a configuration boolean. A missing value (`None`) means true; an
/// empty string means false. The words `true`, `yes`, `on`, `false`, `no`, and
/// `off` are case-insensitive. Otherwise the value must parse as a signed
/// 32-bit configuration integer, with zero meaning false and nonzero true.
///
/// Borrows the input only for this call and returns an owned scalar or error.
/// Pins libgit2's runtime before calling C, including on error paths.
///
/// ```compile_fail,E0308
/// use libgit2::libgit2::config::config::parse_bool;
/// // Only NUL-terminated strings can cross the C boundary.
/// parse_bool(Some("true"));
/// ```
///
/// Wraps: `git_config_parse_bool`
///
/// # Errors
///
/// Fails with libgit2's error, of class [`ErrorClass::Config`], for a value that is neither a
/// boolean word nor an integer, or when libgit2 cannot be initialised.
pub fn parse_bool(value: Option<&CStr>) -> Result<bool, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut out = 0;
    // SAFETY: `out` is one writable, aligned scalar; `value` is NULL or a live
    // NUL-terminated string borrowed read-only for this call. C retains neither
    // pointer. The pinned runtime keeps error reporting's TLS key live.
    let rc = unsafe {
        let value = value.map_or(ptr::null(), CStr::as_ptr);
        ffi::git_config_parse_bool(&raw mut out, value)
    };
    if rc < 0 { Err(last()) } else { Ok(out != 0) }
}

/// Parses a signed 32-bit configuration integer, rejecting values outside its
/// range. Uses [`parse_i64`]'s syntax, including binary multiplier suffixes.
/// `None` and empty strings are errors.
///
/// Borrows the input only for this call and returns an owned scalar or error.
/// Pins libgit2's runtime before calling C, including on error paths.
///
/// Wraps: `git_config_parse_int32`
///
/// # Errors
///
/// Fails with libgit2's error, of class [`ErrorClass::Config`], for `None`, an empty or malformed
/// value, or one outside the `i32` range, or when libgit2 cannot be initialised.
pub fn parse_i32(value: Option<&CStr>) -> Result<i32, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut out = 0;
    // SAFETY: `out` is one writable, aligned i32; `value` is NULL or a live
    // NUL-terminated string borrowed read-only for this call. C retains neither
    // pointer. The pinned runtime keeps error reporting's TLS key live.
    let rc = unsafe {
        let value = value.map_or(ptr::null(), CStr::as_ptr);
        ffi::git_config_parse_int32(&raw mut out, value)
    };
    if rc < 0 { Err(last()) } else { Ok(out) }
}

/// Parses a signed 64-bit configuration integer: optional leading whitespace
/// and sign, decimal digits (octal with a leading `0`, hexadecimal with `0x`),
/// and an optional case-insensitive `k`, `m`, or `g` suffix multiplying by
/// 1024, 1048576, or 1073741824. Trailing characters and overflow are errors.
/// `None` and empty strings are errors.
///
/// Borrows the input only for this call and returns an owned scalar or error.
/// Pins libgit2's runtime before calling C, including on error paths, and clears
/// the thread's last error first, so a success leaves none.
///
/// Wraps: `git_config_parse_int64`
///
/// # Errors
///
/// libgit2 fails on extra characters after a recognized suffix (e.g. `1kb`)
/// without setting an error of its own, leaving whatever an earlier call set;
/// this wrapper reports the failure as libgit2 reports every other one,
/// `failed to parse '<value>' as an integer` in [`ErrorClass::Config`].
///
/// Fails, as well, for `None`, an empty or malformed value, or overflow, or when libgit2 cannot be
/// initialised.
pub fn parse_i64(value: Option<&CStr>) -> Result<i64, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut out = 0;
    clear();
    // SAFETY: `out` is one writable, aligned i64; `value` is NULL or a live
    // NUL-terminated string borrowed read-only for this call. C retains neither
    // pointer. The pinned runtime keeps error reporting's TLS key live.
    let rc = unsafe {
        let value = value.map_or(ptr::null(), CStr::as_ptr);
        ffi::git_config_parse_int64(&raw mut out, value)
    };
    if rc < 0 {
        // The error state was cleared before the call, so `GIT_ERROR_NONE` is
        // libgit2's silence: text after a suffix, the one failure that sets no error.
        if last().class() == Some(ErrorClass::None) {
            set(ErrorClass::Config, &integer_parse_error(value));
        }
        return Err(last());
    }
    Ok(out)
}

/// The message `git_config_parse_int64` sets for a value it cannot parse.
fn integer_parse_error(value: Option<&CStr>) -> CString {
    let value = value.map_or(b"(null)".as_slice(), CStr::to_bytes);
    let message = [b"failed to parse '".as_slice(), value, b"' as an integer"].concat();
    CString::new(message).expect("a C string's bytes and the quotes hold no NUL")
}

/// A configuration priority, including application-defined integer levels.
///
/// Higher numbers are read first. [`HIGHEST`](Self::HIGHEST) is a query selector,
/// not a backend priority. Unknown C values are preserved, not transmuted into a
/// closed enum. Backend-registration wrappers must validate their input range:
/// C's priority comparators subtract signed integers and can overflow when mixing
/// extreme negative and positive levels.
///
/// Wraps: `git_config_level_t`
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConfigLevel(ffi::git_config_level_t);

impl ConfigLevel {
    /// First application-specific priority; larger priorities are also supported.
    pub const APP: Self = Self(ffi::GIT_CONFIG_LEVEL_APP);
    /// The user's global configuration, `~/.gitconfig`.
    pub const GLOBAL: Self = Self(ffi::GIT_CONFIG_LEVEL_GLOBAL);
    /// Select the highest loaded priority when querying, not when adding a backend.
    pub const HIGHEST: Self = Self(ffi::GIT_CONFIG_HIGHEST_LEVEL);
    /// The repository's own configuration, `$GIT_DIR/config`.
    pub const LOCAL: Self = Self(ffi::GIT_CONFIG_LEVEL_LOCAL);
    /// The system-wide configuration, `$(prefix)/etc/gitconfig`.
    pub const SYSTEM: Self = Self(ffi::GIT_CONFIG_LEVEL_SYSTEM);
    /// The worktree's own configuration, `$GIT_DIR/config.worktree`.
    pub const WORKTREE: Self = Self(ffi::GIT_CONFIG_LEVEL_WORKTREE);
    /// The XDG configuration, `$XDG_CONFIG_HOME/git/config`.
    pub const XDG: Self = Self(ffi::GIT_CONFIG_LEVEL_XDG);

    /// Preserves an integer supplied by C, including custom priorities.
    #[must_use]
    pub const fn from_raw(value: ffi::git_config_level_t) -> Self {
        Self(value)
    }

    /// The C value, for interoperability.
    #[must_use]
    pub const fn raw(self) -> ffi::git_config_level_t {
        self.0
    }
}

impl From<ffi::git_config_level_t> for ConfigLevel {
    fn from(value: ffi::git_config_level_t) -> Self {
        Self::from_raw(value)
    }
}

impl From<ConfigLevel> for ffi::git_config_level_t {
    fn from(value: ConfigLevel) -> Self {
        value.raw()
    }
}

/// How a configuration mapping recognizes a value. Integer mappings parse the
/// input directly; boolean and string mappings return the entry's mapped value.
/// Unknown C values are rejected instead of constructing an invalid Rust enum.
///
/// Wraps: `git_configmap_t`
/// Handwritten
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum MapType {
    /// Matches a boolean false value (`GIT_CONFIGMAP_FALSE`).
    False,
    /// Matches a boolean true value, or a valueless entry (`GIT_CONFIGMAP_TRUE`).
    True,
    /// Matches an integer, returning it parsed (`GIT_CONFIGMAP_INT32`).
    Int32,
    /// Matches the rule's string, case-insensitively (`GIT_CONFIGMAP_STRING`).
    String,
}

impl MapType {
    /// Every mapping type, in discriminator order.
    pub const ALL: [Self; 4] = [Self::False, Self::True, Self::Int32, Self::String];

    /// The C discriminator of this mapping type.
    #[must_use]
    pub const fn raw(self) -> ffi::git_configmap_t {
        match self {
            Self::False => ffi::GIT_CONFIGMAP_FALSE,
            Self::True => ffi::GIT_CONFIGMAP_TRUE,
            Self::Int32 => ffi::GIT_CONFIGMAP_INT32,
            Self::String => ffi::GIT_CONFIGMAP_STRING,
        }
    }

    /// The mapping type of the C discriminator `raw`; `None` for a value the header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_configmap_t) -> Option<Self> {
        match raw {
            ffi::GIT_CONFIGMAP_FALSE => Some(Self::False),
            ffi::GIT_CONFIGMAP_TRUE => Some(Self::True),
            ffi::GIT_CONFIGMAP_INT32 => Some(Self::Int32),
            ffi::GIT_CONFIGMAP_STRING => Some(Self::String),
            _ => None,
        }
    }
}

impl From<MapType> for ffi::git_configmap_t {
    fn from(value: MapType) -> Self {
        value.raw()
    }
}

impl<'a, 'r> ConfigMut<'a, 'r> {
    /// Locks the writer backend and retains this exclusive borrow until the
    /// transaction drops. Dropping aborts. The owner does not expose a mutable
    /// config handle: that would permit recursively locking the same C backend.
    ///
    /// Wraps: `git_config_lock`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code: [`ErrorCode::ReadOnly`] when there is no writer
    /// (an empty or read-only config), [`ErrorCode::Locked`] when another writer
    /// holds the file's lock, a backend's own code otherwise.
    pub fn lock(
        self,
    ) -> Result<crate::libgit2::refs::transaction::ConfigTransaction<'a, 'r>, ConfigError> {
        crate::libgit2::refs::transaction::ConfigTransaction::lock(self)
    }
}

/// The failure of a configuration routine: the code it returned, and the error it
/// left as the thread's last libgit2 error, copied out.
///
/// The code tells the failures apart: [`ErrorCode::ReadOnly`] when a config has no
/// writable backend, [`ErrorCode::Locked`] when another writer holds a backend's
/// lock, [`ErrorCode::Generic`] for the rest; a custom backend may return any code.
///
/// Handwritten
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ConfigError {
    code: c_int,
    error: ErrorBuf,
}

impl ConfigError {
    /// The failure a routine reported by returning `rc`, with the thread's last
    /// error. A non-negative `rc` is reported as `GIT_ERROR`.
    pub(crate) fn from_libgit2(rc: c_int) -> Self {
        Self {
            code: if rc < 0 { rc } else { ErrorCode::Generic.raw() },
            error: last(),
        }
    }

    /// A failure in `class` described by `message`, returned as `GIT_ERROR`: what a
    /// [`CustomBackend`](crate::libgit2::config::sys::config::CustomBackend) reports.
    #[must_use]
    pub fn new(class: ErrorClass, message: &CStr) -> Self {
        Self {
            code: ErrorCode::Generic.raw(),
            error: ErrorBuf::new(class.raw(), message),
        }
    }

    /// The same failure, returned as `code` ([`ErrorCode::NotFound`], say).
    /// [`ErrorCode::Ok`] would read as success, so the error then keeps its code.
    #[must_use]
    pub fn with_code(mut self, code: ErrorCode) -> Self {
        if code != ErrorCode::Ok {
            self.code = code.raw();
        }
        self
    }

    /// Makes this the thread's last libgit2 error, verbatim (`git_error_set_str`), for
    /// a callback to return its code. When libgit2 cannot be initialised the error is
    /// not stored; the code is returned either way.
    pub(crate) fn raise(&self) -> c_int {
        if runtime_pinned() {
            restore_last(&self.error);
        }
        self.code
    }

    /// The returned code; `None` for one the header does not name
    /// ([`raw_code`](Self::raw_code) has it).
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
        self.error.class()
    }

    /// The error's class as libgit2 stored it.
    #[must_use]
    pub fn raw_class(&self) -> c_int {
        self.error.raw_class()
    }

    /// The error message.
    #[must_use]
    pub fn message(&self) -> &CStr {
        self.error.message()
    }

    /// The error, without the code.
    #[must_use]
    pub fn error(&self) -> &ErrorBuf {
        &self.error
    }
}

/// The error, without the code.
impl From<ConfigError> for ErrorBuf {
    fn from(error: ConfigError) -> Self {
        error.error
    }
}

impl fmt::Debug for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("ConfigError");
        match self.code() {
            Some(code) => s.field("code", &code),
            None => s.field("code", &self.code),
        };
        s.field("error", &self.error).finish()
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for ConfigError {}

define_ctype!(
    /// A C configuration entry. Its strings belong to the backend's retained
    /// storage, not individually to this struct. C's lookups can return the same
    /// entry, so safe owners are shared and never expose mutable handles.
    /// Iterator/callback entries are borrowed through `EntryRef`, never freed.
    /// No public C up-ref or standalone deep-copy primitive exists.
    /// Every non-NULL string must remain NUL-terminated and immutable for the
    /// entry handle's lifetime; C list ownership enforces this for built-in backends.
    /// Neither entries nor their owners assert `Send`/`Sync`: a custom backend's
    /// release callback may be thread-bound.
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::EntryArc;
    /// fn mutate(mut entry: EntryArc) { entry.as_mut(); }
    /// ```
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::EntryArc;
    /// fn escape(entry: EntryArc) {
    ///     let name = entry.as_ref().name();
    ///     drop(entry);
    ///     assert!(name.is_some());
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::EntryArc;
    /// fn send(entry: EntryArc) { std::thread::spawn(move || drop(entry)); }
    /// ```
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::EntryArc;
    /// fn clone(entry: EntryArc) { let _ = entry.clone(); }
    /// ```
    ///
    /// Wraps: git_config_entry
    Entry,
    EntryRef,
    EntryMut,
    ffi::git_config_entry
);

/// Calls the entry's backend-provided release callback through C. For built-in
/// backends this releases a reference to the entire immutable entry list; it
/// does not necessarily free the entry immediately. Never use this policy for
/// a standalone `git_config_entry` or an iterator/callback's borrowed entry:
/// C casts to the larger `git_config_backend_entry` to find the callback.
///
/// Wraps: `git_config_entry_free`
#[derive(Clone, Copy, Debug, Default)]
pub struct EntryFree;
impl_cdrop!(EntryFree, Entry, ffi::git_config_entry_free);

/// One acquired entry reference. Not `Clone`: C publishes no entry up-ref.
/// The backing storage stays alive independently of the config/repository.
pub type EntryArc = CArc<Entry, EntryFree>;

impl<'a> ConfigRef<'a, '_> {
    /// Creates an iterator borrowing this config, without retaining a reference.
    ///
    /// Wraps: `git_config_iterator_new`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when a backend cannot reread its changed file or the
    /// iterator cannot be allocated.
    ///
    /// # Panics
    ///
    /// Panics, with `successful iterator constructor returned NULL`, only if libgit2 breaks its
    /// contract.
    pub fn iterator(&self) -> Result<IteratorBox<'a>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: this handle keeps config/repository live for 'a; writable out
        // slot; C constructs a fresh complete allocation or cleans up failure.
        let rc = unsafe { ffi::git_config_iterator_new(&raw mut out, self.as_ptr()) };
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
    /// Wraps: `git_config_iterator_glob_new`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid pattern, when a backend cannot reread its
    /// changed file, or when the iterator cannot be allocated.
    ///
    /// # Panics
    ///
    /// Panics, with `successful iterator constructor returned NULL`, only if libgit2 breaks its
    /// contract.
    pub fn iterator_matching(&self, regexp: Option<&CStr>) -> Result<IteratorBox<'a>, ConfigError> {
        let mut out = ptr::null_mut();
        // SAFETY: live config and writable out; regexp is NUL-terminated and
        // borrowed only for the call. C frees failed regex construction.
        let rc = unsafe {
            ffi::git_config_iterator_glob_new(
                &raw mut out,
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
    /// C's name-selection constructor runs either way, so its leak of the inner
    /// iterator when its own allocation fails (`config.c:1139`) remains, with or
    /// without a regex.
    ///
    /// Wraps: `git_config_multivar_iterator_new`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid name or pattern, when a backend cannot
    /// reread its changed file, or when the iterator cannot be allocated.
    ///
    /// # Panics
    ///
    /// Panics, with `successful iterator constructor returned NULL`, only if libgit2 breaks its
    /// contract.
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
                &raw mut out,
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
            Some(regexp) => crate::libgit2::config::sys::config::value_filter::new(inner, regexp),
        }
    }
}

impl IteratorMut<'_, '_> {
    /// Advances to a public configuration entry. The entry is borrowed, never
    /// acquired: it remains valid only until this exclusive reborrow ends.
    /// `None` is libgit2's end-of-iteration status. Other errors retain their
    /// code and message, and do not prevent retrying (C can move to a different
    /// backend after a refresh failure).
    ///
    /// Unlike [`advance`](Self::advance), this exposes only the public entry,
    /// not the enclosing backend entry or its release dispatch.
    ///
    /// The entry cannot survive another advance, even through the backend API:
    ///
    /// ```compile_fail,E0499
    /// use libgit2::libgit2::config::config::Config;
    /// let config = Config::new().unwrap();
    /// let mut iter = config.as_ref().iterator().unwrap();
    /// let mut m = iter.as_mut();
    /// let entry = m.advance_entry().unwrap().unwrap();
    /// let _ = m.advance();
    /// let _ = entry.name();
    /// ```
    ///
    /// Nor can its owner be dropped first:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::config::config::Config;
    /// let config = Config::new().unwrap();
    /// let mut iter = config.as_ref().iterator().unwrap();
    /// let mut m = iter.as_mut();
    /// let entry = m.advance_entry().unwrap().unwrap();
    /// drop(iter);
    /// let _ = entry.value();
    /// ```
    ///
    /// Wraps: `git_config_next`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when a backend cannot refresh or read its next entry;
    /// the iterator may be advanced again after such a failure. The end of the iteration is
    /// `Ok(None)`, not an error.
    ///
    /// # Panics
    ///
    /// Panics, with `successful iterator advance returned NULL`, only if libgit2 breaks its
    /// contract.
    pub fn advance_entry(&mut self) -> Result<Option<EntryRef<'_>>, ConfigError> {
        let mut entry = ptr::null_mut();
        // SAFETY: the exclusive handle denotes a complete live iterator with
        // its allocation-matched callback; out is writable. No entry from an
        // earlier advance remains lent across this exclusive reborrow.
        let rc = unsafe { ffi::git_config_next(&raw mut entry, self.as_mut_ptr()) };
        if rc == ffi::GIT_ITEROVER {
            return Ok(None);
        }
        if rc != 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        // SAFETY: success lends an initialized immutable public entry, kept
        // live by the iterator until next advance/free. The returned lifetime
        // freezes this mutable handle, excluding both operations.
        let entry = unsafe { EntryRef::from_ptr(entry) };
        Ok(Some(
            entry.expect("successful iterator advance returned NULL"),
        ))
    }
}

impl ConfigRef<'_, '_> {
    /// Acquires an immutable entry reference, released on drop. The key is
    /// normalized by C and borrowed only during this call. Missing keys and
    /// invalid names retain C's error code and message.
    ///
    /// The result is `git_config_get_entry`'s: the value of the highest-priority
    /// backend holding the key, or the failure of a backend above it that cannot
    /// reread its changed file. C reaches that failure through an entry pointer
    /// the backend never wrote (`get_entry`, `config.c:811`), so the lookup is
    /// composed from routines that report it cleanly: `name`'s multivar iterator
    /// rereads every backend, lowest priority first, and a snapshot, whose
    /// backends never reread, then hands out the entry. Two differences follow:
    /// each lookup copies the configuration, so repeated lookups return distinct
    /// entries; and a backend below the key's also rereads, so a failure there is
    /// spent on this lookup, which succeeds, and a later one reads its old entries
    /// where C would report it.
    ///
    /// libgit2 mishandles some allocation failures on this path: copying a file
    /// backend's entries can corrupt its heap, as for [`snapshot`](Self::snapshot),
    /// and one failure leaks an iterator.
    ///
    /// Wraps: `git_config_get_entry`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`] when no backend holds `name`, and with libgit2's code and
    /// error for an invalid name, a backend that cannot reread its changed file, or an allocation
    /// failure.
    ///
    /// # Panics
    ///
    /// Panics, with `successful entry lookup returned NULL`, only if libgit2 breaks its contract.
    pub fn entry(&self, name: &CStr) -> Result<EntryArc, ConfigError> {
        let before = last();
        if let Some(failure) = self.reread_through(name)? {
            return Err(failure);
        }
        let snapshot = self.snapshot()?;
        let mut out = ptr::null_mut();
        // SAFETY: the snapshot is live for the call; its backends never reread,
        // so `get` fails only as a found or missing key; name is NUL-terminated;
        // out is an exclusive writable slot. Success acquires one reference.
        let rc =
            unsafe { ffi::git_config_get_entry(&raw mut out, snapshot.as_c_ptr(), name.as_ptr()) };
        if rc < 0 {
            return Err(ConfigError::from_libgit2(rc));
        }
        if last() != before {
            // A lower backend's failure was spent: leave the thread's error as
            // a successful C lookup does.
            restore_last(&before);
        }
        // SAFETY: success returned one reference paired with entry_free; the
        // entry pins the snapshot's list, not the snapshot dropped below.
        let owner = unsafe { EntryArc::from_c(out) };
        Ok(owner.expect("successful entry lookup returned NULL"))
    }

    /// Rereads every backend, lowest priority first, through `name`'s multivar
    /// iterator, which reports a backend that cannot reread without forming an
    /// entry. Returns the failure of the highest-priority backend that failed
    /// above every backend holding `name`: C's lookup stops there. An invalid
    /// name fails first, as in C.
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "callers hold a reference to the value"
    )]
    fn reread_through(&self, name: &CStr) -> Result<Option<ConfigError>, ConfigError> {
        let mut iter = self.multivar_iterator(name, None)?;
        let mut failure = None;
        // Built-in backends fail only while opening their iterator, after which
        // libgit2 moves on to the next one, so this ends after every backend.
        loop {
            match iter.as_mut().advance() {
                Ok(Some(_)) => failure = None,
                Ok(None) => break,
                Err(error) => failure = Some(error),
            }
        }
        Ok(failure)
    }
}

/// Puts `before` back as the thread's last error, verbatim. `set` would format
/// it again, and for `GIT_ERROR_OS` libgit2 appends `errno`'s description.
fn restore_last(before: &ErrorBuf) {
    if before.class() == Some(ErrorClass::None) {
        clear();
    } else {
        // SAFETY: the caller has just called libgit2, so the runtime is pinned;
        // the message is NUL-terminated and copied during the call. The class is
        // stored, never interpreted, so an unknown one is restored as it was.
        unsafe { ffi::git_error_set_str(before.raw_class(), before.message().as_ptr()) };
    }
}

impl ConfigRef<'_, '_> {
    /// Maps the highest-priority value of `name` through the ordered table.
    /// A valueless entry is passed as `None`, not as an empty string.
    ///
    /// Composed as `git_config_get_mapped` is, from [`entry`](Self::entry) and
    /// [`lookup_map_value`]: C's direct lookup forms an uninitialized entry
    /// pointer on a backend refresh error, and C's mapper formats a valueless
    /// entry through `%s` when no rule matches it. The snapshot costs,
    /// lower-level refresh differences, and C allocation-failure limitations of
    /// `entry` apply here. Mapping retains C's first-match semantics and error
    /// reporting.
    ///
    /// Wraps: `git_config_get_mapped`
    ///
    /// # Errors
    ///
    /// Fails as [`entry`](Self::entry) does, and as [`lookup_map_value`] does when no rule matches
    /// the value.
    pub fn get_mapped(&self, name: &CStr, maps: &[ConfigMap<'_>]) -> Result<i32, ConfigError> {
        let entry = self.entry(name)?;
        lookup_map_value(maps, entry.as_ref().value())
    }
}

/// Maps `value` using the first matching rule in `maps`. Boolean rules return
/// their mapped value; integer rules return the parsed input, ignoring their
/// mapped value. String rules compare case-insensitively. An empty table fails.
/// `None` is a valueless setting (boolean true), distinct from an empty string
/// (boolean false). The table and strings are borrowed only during this call.
///
/// Pins libgit2's runtime before calling C, including its error paths. C's
/// speculative parsers may leave a last error even when a later rule succeeds;
/// this wrapper preserves that behavior.
///
/// Only a boolean-true rule matches `None`. Without one, C would report the
/// failure by formatting the NULL value through `%s`, which ISO C leaves
/// undefined; the wrapper reports it itself, spelling the value `(null)` as
/// libgit2's parsers and glibc do.
///
/// Wraps: `git_config_lookup_map_value`
///
/// # Errors
///
/// Fails with libgit2's code and error, of class [`ErrorClass::Config`], when no rule matches
/// `value`, the table is empty, or libgit2 cannot be initialised.
pub fn lookup_map_value(maps: &[ConfigMap<'_>], value: Option<&CStr>) -> Result<i32, ConfigError> {
    if !runtime_pinned() {
        return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
    }
    if value.is_none() && !maps.iter().any(|m| m.as_ref().map_type() == MapType::True) {
        set(ErrorClass::Config, c"failed to map '(null)'");
        return Err(ConfigError::from_libgit2(ffi::GIT_ERROR));
    }
    let mut out = 0;
    // SAFETY: out is writable; the transparent mapping slice covers exactly
    // len initialized records with valid string/discriminator pairs. All
    // strings are NULL or immutable live C strings and C retains nothing.
    let rc = unsafe {
        ffi::git_config_lookup_map_value(
            &raw mut out,
            maps.as_ptr().cast(),
            maps.len(),
            value.map_or(ptr::null(), CStr::as_ptr),
        )
    };
    if rc < 0 {
        Err(ConfigError::from_libgit2(rc))
    } else {
        Ok(out)
    }
}

impl ConfigRef<'_, '_> {
    /// Visits every entry, including duplicates, in increasing backend priority.
    /// Entries and their strings are borrowed only for each callback invocation.
    /// A callback stop is distinct from a backend error, even for negative codes
    /// or `GIT_ITEROVER` (which C otherwise converts to success).
    /// Panics resume after C has released its iterator; the callback can be reused.
    /// Independent nested reads are permitted; mutation and dropping the config
    /// remain excluded by this handle's borrow.
    ///
    /// Entry borrows cannot escape a walk:
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::{Config, EntryRef, ForeachCallback};
    /// use std::ops::ControlFlow;
    /// let config = Config::new().unwrap();
    /// let mut kept = None;
    /// let mut f = |entry: EntryRef<'_>| {
    ///     kept = Some(entry);
    ///     ControlFlow::Continue(())
    /// };
    /// config.as_ref().foreach(&mut ForeachCallback::new(&mut f)).unwrap();
    /// ```
    ///
    /// A callback cannot drop or mutate the configuration being walked:
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::{Config, EntryRef, ForeachCallback};
    /// use std::ops::ControlFlow;
    /// let mut config = Config::new().unwrap();
    /// let view = config.as_ref();
    /// let mut f = |_: EntryRef<'_>| {
    ///     config.as_mut().set_bool(c"a.b", true).unwrap();
    ///     ControlFlow::Continue(())
    /// };
    /// view.foreach(&mut ForeachCallback::new(&mut f)).unwrap();
    /// ```
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::config::config::{Config, EntryRef, ForeachCallback};
    /// use std::ops::ControlFlow;
    /// let mut config = Some(Config::new().unwrap());
    /// let view = config.as_ref().unwrap().as_ref();
    /// let mut f = |_: EntryRef<'_>| {
    ///     drop(config.take());
    ///     ControlFlow::Continue(())
    /// };
    /// view.foreach(&mut ForeachCallback::new(&mut f)).unwrap();
    /// ```
    ///
    /// As with C enumeration, file backends copy their entry lists. libgit2's
    /// previously documented list-copy allocation-failure defects also apply here.
    ///
    /// Wraps: `git_config_foreach`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error when a backend cannot reread its changed file or list
    /// its entries.
    pub fn foreach(
        &self,
        callback: &mut ForeachCallback<'_>,
    ) -> Result<ControlFlow<NonZero<c_int>>, ConfigError> {
        let rc = callback.walk_with(|cb, payload| {
            // SAFETY: the handle keeps the config/repository live; C calls cb
            // synchronously with immutable iterator-owned entries and the exclusive
            // payload supplied by walk_with. C retains nothing and frees its iterator.
            unsafe { ffi::git_config_foreach(self.as_ptr(), cb, payload) }
        });
        foreach_result(rc, callback)
    }

    /// Visits entries whose normalized **names** match the case-sensitive C regex.
    /// `None` visits every entry; an empty pattern matches every name. Section and
    /// variable names are lower-case, but subsection case is preserved.
    /// No matches is success. Callback stops, panics and list-copy limitations are
    /// handled as in [`foreach`](Self::foreach).
    ///
    /// Wraps: `git_config_foreach_match`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid pattern, or when a backend cannot reread
    /// its changed file or list its entries; a callback's stop is not an error.
    pub fn foreach_match(
        &self,
        regexp: Option<&CStr>,
        callback: &mut ForeachCallback<'_>,
    ) -> Result<ControlFlow<NonZero<c_int>>, ConfigError> {
        let rc = callback.walk_with(|cb, payload| {
            // SAFETY: live config/repository and optional immutable NUL pattern;
            // synchronous exclusive callback payload, immutable borrowed entries.
            // C compiles/disposes its regex and frees its iterator before returning.
            unsafe {
                ffi::git_config_foreach_match(
                    self.as_ptr(),
                    regexp.map_or(ptr::null(), CStr::as_ptr),
                    cb,
                    payload,
                )
            }
        });
        foreach_result(rc, callback)
    }

    /// Visits all values of the normalized key, across increasing backend priority.
    /// A missing key is `NotFound`, unlike the successful empty `foreach` walk.
    /// Valueless settings arrive as `EntryRef::value() == None`, distinct from `""`.
    /// Callback stops, panics and list-copy limitations are as in `foreach`.
    ///
    /// `regexp` keeps only values matching the case-sensitive C regex (`None`
    /// keeps all); no matching value is `NotFound`, an invalid pattern
    /// `InvalidSpec`. C's own value filter calls `strlen(NULL)` for a valueless
    /// setting and leaks the normalized key on invalid patterns, so a filtered
    /// walk is composed as C composes it, over the same iterator with
    /// [`multivar_iterator`](Self::multivar_iterator)'s PCRE2 value filter: it
    /// skips valueless settings and leaks nothing, and otherwise matches, calls
    /// back and reports errors as C does.
    ///
    /// Wraps: `git_config_get_multivar_foreach`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid name, [`ErrorCode::InvalidSpec`] for an
    /// invalid pattern, and [`ErrorCode::NotFound`] when no value matches.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: the trampoline is always set"
    )]
    pub fn foreach_multivar(
        &self,
        name: &CStr,
        regexp: Option<&CStr>,
        callback: &mut ForeachCallback<'_>,
    ) -> Result<ControlFlow<NonZero<c_int>>, ConfigError> {
        let Some(regexp) = regexp else {
            let rc = callback.walk_with(|cb, payload| {
                // SAFETY: live config/repository and immutable NUL key; NULL disables
                // C's unsafe value-regex path. The exclusive callback payload and
                // iterator-owned entries are borrowed synchronously and never retained.
                // C releases its iterator and copied key before returning.
                unsafe {
                    ffi::git_config_get_multivar_foreach(
                        self.as_ptr(),
                        name.as_ptr(),
                        ptr::null(),
                        cb,
                        payload,
                    )
                }
            });
            return foreach_result(rc, callback);
        };
        let rc = callback.walk_with(|cb, payload| {
            let cb = cb.expect("the trampoline is always set");
            let mut iter = match self.multivar_iterator(name, Some(regexp)) {
                Ok(iter) => iter,
                // The thread's error is still the constructor's, as in C.
                Err(error) => return error.raw_code(),
            };
            let mut found = false;
            let rc = loop {
                match iter.as_mut().advance() {
                    Ok(Some(entry)) => {
                        found = true;
                        // SAFETY: the iterator pins this immutable entry until the
                        // next advance; payload is the exclusive handle walk_with
                        // lends for this synchronous call. Neither is retained.
                        let rc = unsafe { cb(entry.entry().as_ptr(), payload) };
                        if rc != 0 {
                            break after_callback(rc, "git_config_get_multivar_foreach");
                        }
                    }
                    Ok(None) => break 0,
                    // The thread's error is still the failing backend's.
                    Err(error) => break error.raw_code(),
                }
            };
            drop(iter);
            if !found && rc == 0 {
                // C's `config_error_notfound`, quoting the caller's spelling.
                let message = [b"config value '", name.to_bytes(), b"' was not found"].concat();
                let message = CString::new(message).expect("a C string has no inner NUL");
                set(ErrorClass::Config, &message);
                return ErrorCode::NotFound.raw();
            }
            rc
        });
        foreach_result(rc, callback)
    }
}

impl BackendMut<'_> {
    /// Visits this opened backend's entries whose normalized names match the C
    /// regex. `None` and an empty pattern visit all entries, including duplicates.
    /// Entries are lent only for the callback; the backend remains exclusively
    /// borrowed throughout. Stops preserve their nonzero code, and panics resume
    /// after both C's regex and the Rust-held iterator have been released.
    ///
    /// C leaks its regex on iterator-construction failure and hides negative next
    /// statuses. A prepared iterator adapter keeps C's matching/callback behavior
    /// while releasing resources on every path and reporting those backend errors.
    /// Iterator construction precedes C regex compilation in this wrapper. A
    /// nameless entry is allowed without a pattern, but rejected when matching
    /// would pass its NULL name to C's regex engine.
    ///
    /// Wraps: `git_config_backend_foreach_match`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's code and error for an invalid pattern, a nameless entry met while
    /// matching, or when the backend cannot list its entries; a callback's stop is not an error.
    pub fn foreach_match(
        &mut self,
        regexp: Option<&CStr>,
        callback: &mut ForeachCallback<'_>,
    ) -> Result<ControlFlow<NonZero<c_int>>, ConfigError> {
        let (rc, failure) = callback.walk_with(|cb, payload| {
            let inner = match self.iterator() {
                Ok(inner) => inner,
                Err(error) => return (error.raw_code(), Some(error)),
            };
            let (rc, mut state) = PreparedBackendWalk::run(inner, regexp.is_some(), |backend| {
                // SAFETY: stable Rust-owned ABI bases with matched adapter callbacks.
                // C reads only backend.iterator, calls next/free synchronously, and
                // retains nothing. The prepared owner pins entries and backend; the
                // callback payload is exclusive and the optional NUL pattern is live.
                unsafe {
                    ffi::git_config_backend_foreach_match(
                        backend,
                        regexp.map_or(ptr::null(), CStr::as_ptr),
                        cb,
                        payload,
                    )
                }
            });
            // The inner iterator is dropped before walk_with can resume a panic.
            (rc, state.failure.take())
        });
        if callback.stopped().is_none()
            && let Some(error) = failure
        {
            return Err(error);
        }
        foreach_result(rc, callback)
    }
}

/// Private, borrowed ABI adapter; no C release owns these stack bases. Its Rust
/// state alone owns the foreign iterator via ffibox. It is never exposed as an
/// opened Backend or adopted by an owner, and is not moved during the C call.
/// Handwritten
#[repr(C)]
struct PreparedBackendWalk<'c> {
    backend: ffi::git_config_backend,
    iterator: ffi::git_config_iterator,
    state: PreparedWalkState<'c>,
}
struct PreparedWalkState<'c> {
    inner: IteratorBox<'c>,
    failure: Option<ConfigError>,
    require_name: bool,
}
impl<'c> PreparedBackendWalk<'c> {
    /// Lays the adapter out in this frame and lends `walk` its backend base, the
    /// only pointer C receives. Returns the walk's status and the Rust state, which
    /// still owns the real iterator, after `walk` returns. Unit tests drive the
    /// same setup under Miri, so the provenance of the base is checked there too.
    fn run(
        inner: IteratorBox<'c>,
        require_name: bool,
        walk: impl FnOnce(*mut ffi::git_config_backend) -> c_int,
    ) -> (c_int, PreparedWalkState<'c>) {
        let mut prepared = PreparedBackendWalk {
            backend: ffi::git_config_backend {
                version: ffi::GIT_CONFIG_BACKEND_VERSION,
                iterator: Some(Self::iterator),
                ..Default::default()
            },
            iterator: ffi::git_config_iterator {
                backend: ptr::null_mut(),
                flags: 0,
                next: Some(Self::next),
                free: Some(Self::free),
            },
            state: PreparedWalkState {
                inner,
                failure: None,
                require_name,
            },
        };
        let p = ptr::from_mut(&mut prepared);
        // SAFETY: project and initialize through the same raw provenance C
        // will use; a fresh Rust reborrow would invalidate that retained tag.
        let backend_ptr = unsafe { addr_of_mut!((*p).iterator.backend) };
        // SAFETY: as above.
        unsafe { backend_ptr.write(p.cast()) };
        let rc = walk(p.cast());
        (rc, prepared.state)
    }

    unsafe extern "C" fn iterator(
        out: *mut *mut ffi::git_config_iterator,
        backend: *mut ffi::git_config_backend,
    ) -> c_int {
        // SAFETY: only this adapter installs the callback; backend is its offset-zero
        // base, out is writable, and the enclosing stable stack object outlives C.
        let value = unsafe { addr_of_mut!((*backend.cast::<Self>()).iterator) };
        // SAFETY: as above.
        unsafe { out.write(value) };
        0
    }

    unsafe extern "C" fn next(
        out: *mut *mut ffi::git_config_backend_entry,
        iterator: *mut ffi::git_config_iterator,
    ) -> c_int {
        let state = {
            // SAFETY: iterator is this adapter's live base, with the enclosing pointer
            // installed before C. State is Rust-owned, exclusively used synchronously;
            // the reference covers no C-owned layout or pointee.
            let backend_ptr = unsafe { addr_of!((*iterator).backend) };
            // SAFETY: as above.
            let p = unsafe { backend_ptr.read().cast::<Self>() };
            // SAFETY: as above; the state is Rust-owned, reached exclusively for the call.
            unsafe { &mut (*p).state }
        };
        let mut m = state.inner.as_mut();
        match m.advance() {
            Ok(Some(entry)) => {
                // Entry handles permit nullable names; C's regex matcher does not.
                // No pattern still supports such entries through the typed callback.
                if state.require_name && entry.entry().name().is_none() {
                    set(
                        ErrorClass::Config,
                        c"config backend iterator returned an entry without a name",
                    );
                    let error = ConfigError::from_libgit2(ffi::GIT_EINVALIDSPEC);
                    let rc = error.raw_code();
                    state.failure = Some(error);
                    return rc;
                }
                // SAFETY: out is writable; inner pins this immutable entry through
                // C's callback until next/free, and no Rust entry borrow escapes.
                unsafe { out.write(entry.as_ptr().cast_mut()) };
                0
            }
            Ok(None) => ffi::GIT_ITEROVER,
            Err(error) => {
                let rc = error.raw_code();
                state.failure = Some(error);
                rc
            }
        }
    }

    unsafe extern "C" fn free(_: *mut ffi::git_config_iterator) {
        // C borrows the stack iterator. The enclosing Rust state releases the
        // real owner immediately after C returns (also on callback panic).
    }
}

/// `git_error_set_after_callback` for a walk composed in Rust: a nonzero stop
/// leaves "<action> callback returned <rc>" unless the thread has an error.
fn after_callback(rc: c_int, action: &str) -> c_int {
    if last().class() == Some(ErrorClass::None) {
        let message = CString::new(format!("{action} callback returned {rc}"))
            .expect("the action has no inner NUL");
        set(ErrorClass::Callback, &message);
    }
    rc
}

/// C folds a callback's `GIT_ITEROVER` into success, so consult the callback state
/// first. Never mistake a callback's negative stop for a backend error.
fn foreach_result(
    rc: c_int,
    callback: &ForeachCallback<'_>,
) -> Result<ControlFlow<NonZero<c_int>>, ConfigError> {
    if let Some(code) = callback.stopped() {
        Ok(ControlFlow::Break(code))
    } else if rc != 0 {
        Err(ConfigError::from_libgit2(rc))
    } else {
        Ok(ControlFlow::Continue(()))
    }
}

type ForeachFn<'f> = dyn for<'e> FnMut(EntryRef<'e>) -> ControlFlow<NonZero<c_int>> + 'f;

/// A synchronous enumeration callback borrowing each entry only for its call.
/// The iterator, not this handle or the closure, owns and releases the entry.
/// All C invokers use this same borrowed distribution, including backend and
/// multivar enumeration. Positive and negative nonzero stop codes are retained.
///
/// Panics stop C iteration and resume only once C has returned and cleaned up.
/// The exclusive callback borrow prevents same-handle reentry; a callback may
/// call another independently borrowed callback. The handle is not Send/Sync.
///
/// An entry cannot escape into the closure's captured state:
/// ```compile_fail
/// use libgit2::libgit2::config::config::{EntryRef, ForeachCallback};
/// use std::ops::ControlFlow;
/// let mut kept = None;
/// let mut f = |entry: EntryRef<'_>| {
///     kept = entry.value();
///     ControlFlow::Continue(())
/// };
/// let _ = ForeachCallback::new(&mut f);
/// ```
///
/// Captured mutable state cannot be accessed while the callback is used:
/// ```compile_fail
/// use libgit2::libgit2::config::config::{EntryRef, ForeachCallback};
/// use std::ops::ControlFlow;
/// let mut count = 0;
/// let mut f = |_: EntryRef<'_>| { count += 1; ControlFlow::Continue(()) };
/// let callback = ForeachCallback::new(&mut f);
/// count += 1;
/// let _ = callback.stopped();
/// ```
///
/// Neither concurrent calls nor moving a borrowed callback to another thread
/// is permitted:
/// ```compile_fail
/// use libgit2::libgit2::config::config::{EntryRef, ForeachCallback};
/// use std::ops::ControlFlow;
/// let mut f = |_: EntryRef<'_>| ControlFlow::Continue(());
/// let callback = ForeachCallback::new(&mut f);
/// std::thread::scope(|scope| { scope.spawn(move || drop(callback)); });
/// ```
///
/// Wraps: `git_config_foreach_cb`
/// Handwritten
pub struct ForeachCallback<'f> {
    f: &'f mut ForeachFn<'f>,
    stopped: Option<NonZero<c_int>>,
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> ForeachCallback<'f> {
    /// A callback that calls `f` with each entry, until it returns `Break` with the code to stop
    /// the walk with.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: for<'e> FnMut(EntryRef<'e>) -> ControlFlow<NonZero<c_int>> + 'f,
    {
        Self {
            f,
            stopped: None,
            panic: None,
        }
    }

    /// Calls through the C trampoline and resumes any panic on the Rust side.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: the trampoline is always set"
    )]
    pub fn call(&mut self, entry: EntryRef<'_>) -> ControlFlow<NonZero<c_int>> {
        let rc = self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is always set");
            // SAFETY: entry is live and immutable for the call; payload points
            // to this exclusively borrowed handle, as walk_with supplies it.
            unsafe { callback(entry.as_ptr(), payload) }
        });
        NonZero::new(rc).map_or(ControlFlow::Continue(()), ControlFlow::Break)
    }

    /// The closure's stop code in the last invocation/walk, including `GIT_EUSER`
    /// for a panic. Distinguishes callback stops from C's own errors.
    #[must_use]
    pub fn stopped(&self) -> Option<NonZero<c_int>> {
        self.stopped
    }

    /// Internal synchronous seam for the enumeration wrappers. The caller must
    /// not retain either pointer, call concurrently/reentrantly, or lend an entry
    /// whose strings change during the callback. No reference to self is kept
    /// across foreign callback writes.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_config_foreach_cb, *mut c_void) -> R,
    ) -> R {
        self.stopped = None;
        let result = walk(Some(foreach_with), ptr::from_mut(self).cast());
        if let Some(payload) = self.panic.take() {
            panic::resume_unwind(payload);
        }
        result
    }
}

impl fmt::Debug for ForeachCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForeachCallback")
            .field("stopped", &self.stopped)
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// # Safety
/// payload addresses a live exclusively used `ForeachCallback` on this thread;
/// entry is NULL or a live immutable entry with NULL/immutable NUL strings for
/// this call. Neither pointer is retained or freed here.
unsafe extern "C" fn foreach_with(
    entry: *const ffi::git_config_entry,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: the internal seam gives exclusive access to this live handle.
    let callback = unsafe { &mut *payload.cast::<ForeachCallback<'_>>() };
    if let Some(code) = callback.stopped {
        return code.get();
    }
    // SAFETY: the contract supplies a live immutable entry for this call only;
    // NULL is rejected. The higher-ranked closure cannot retain this borrow.
    let Some(entry) = (unsafe { EntryRef::from_ptr(entry.cast_mut()) }) else {
        let code = NonZero::new(ErrorCode::User.raw()).expect("GIT_EUSER is nonzero");
        callback.stopped = Some(code);
        return code.get();
    };
    let code = match panic::catch_unwind(AssertUnwindSafe(|| (callback.f)(entry))) {
        Ok(ControlFlow::Continue(())) => return 0,
        Ok(ControlFlow::Break(code)) => code,
        Err(payload) => {
            callback.panic = Some(payload);
            NonZero::new(ErrorCode::User.raw()).expect("GIT_EUSER is nonzero")
        }
    };
    callback.stopped = Some(code);
    code.get()
}

impl<'a> EntryRef<'a> {
    /// Normalized key; built-in backends always supply it.
    /// Strings are immutable and tied to this entry borrow, including non-UTF-8.
    ///
    /// Field: `git_config_entry.name`
    #[must_use]
    pub fn name(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let name_ptr = unsafe { &raw const (*self.as_ptr()).name };
        // SAFETY: the live shared handle permits reading this initialized field.
        let value = unsafe { name_ptr.read() };
        if value.is_null() {
            None
        } else {
            // SAFETY: entry storage keeps this NUL-terminated string alive and
            // immutable for the entire handle lifetime, including across refresh.
            Some(unsafe { CStr::from_ptr(value) })
        }
    }

    /// None is an implicit true boolean, distinct from an empty string.
    /// Strings are immutable and tied to this entry borrow, including non-UTF-8.
    ///
    /// Field: `git_config_entry.value`
    #[must_use]
    pub fn value(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let value_ptr = unsafe { &raw const (*self.as_ptr()).value };
        // SAFETY: the live shared handle permits reading this initialized field.
        let value = unsafe { value_ptr.read() };
        if value.is_null() {
            None
        } else {
            // SAFETY: entry storage keeps this NUL-terminated string alive and
            // immutable for the entire handle lifetime, including across refresh.
            Some(unsafe { CStr::from_ptr(value) })
        }
    }

    /// Backend label, typically "file" or "in-memory".
    /// Strings are immutable and tied to this entry borrow, including non-UTF-8.
    ///
    /// Field: `git_config_entry.backend_type`
    #[must_use]
    pub fn backend_type(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let backend_type_ptr = unsafe { &raw const (*self.as_ptr()).backend_type };
        // SAFETY: the live shared handle permits reading this initialized field.
        let value = unsafe { backend_type_ptr.read() };
        if value.is_null() {
            None
        } else {
            // SAFETY: entry storage keeps this NUL-terminated string alive and
            // immutable for the entire handle lifetime, including across refresh.
            Some(unsafe { CStr::from_ptr(value) })
        }
    }

    /// Origin path, absent for a memory backend without an origin.
    /// Strings are immutable and tied to this entry borrow, including non-UTF-8.
    ///
    /// Field: `git_config_entry.origin_path`
    #[must_use]
    pub fn origin_path(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let origin_path_ptr = unsafe { &raw const (*self.as_ptr()).origin_path };
        // SAFETY: the live shared handle permits reading this initialized field.
        let value = unsafe { origin_path_ptr.read() };
        if value.is_null() {
            None
        } else {
            // SAFETY: entry storage keeps this NUL-terminated string alive and
            // immutable for the entire handle lifetime, including across refresh.
            Some(unsafe { CStr::from_ptr(value) })
        }
    }

    /// Field: `git_config_entry.level`
    #[must_use]
    pub fn level(&self) -> ConfigLevel {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let level_ptr = unsafe { &raw const (*self.as_ptr()).level };
        // SAFETY: reads an initialized scalar; ConfigLevel preserves custom values.
        let level = unsafe { level_ptr.read() };
        ConfigLevel::from_raw(level)
    }

    /// Field: `git_config_entry.include_depth`
    #[must_use]
    pub fn include_depth(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let include_depth_ptr = unsafe { &raw const (*self.as_ptr()).include_depth };
        // SAFETY: reads the initialized unsigned scalar through a live handle.
        unsafe { include_depth_ptr.read() }
    }
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for EntryRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("name", &self.name())
            .field("value", &self.value())
            .field("backend_type", &self.backend_type())
            .field("origin_path", &self.origin_path())
            .field("level", &self.level())
            .field("include_depth", &self.include_depth())
            .finish()
    }
}

impl fmt::Debug for EntryMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// A mapping's recognition rule. String mode cannot be constructed with NULL;
/// the other modes need no string and do not interpret `map_value` identically:
/// integer mode returns the parsed input, not the record's mapped value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapMatch<'s> {
    /// A boolean-false rule matched.
    False,
    /// A boolean-true rule matched.
    True,
    /// An integer rule matched.
    Int32,
    /// A string rule matched, with its string.
    String(&'s CStr),
}

impl MapMatch<'_> {
    /// The mapping type of the rule that matched.
    #[must_use]
    pub const fn map_type(self) -> MapType {
        match self {
            Self::False => MapType::False,
            Self::True => MapType::True,
            Self::Int32 => MapType::Int32,
            Self::String(_) => MapType::String,
        }
    }
}

/// Caller-owned mapping record, borrowing its optional string for `'s`.
/// C reads it during lookup and neither retains nor frees it. No C lifecycle
/// primitive exists: moves and clones retain only immutable Rust borrows.
/// Mutation replaces borrowed pointers without freeing either referent.
///
/// Records build in constant context and are `Sync`, so a mapping table can
/// be a `static`, as C's are:
///
/// ```
/// use libgit2::libgit2::config::config::{ConfigMap, MapMatch};
/// static AUTOCRLF: [ConfigMap<'static>; 3] = [
///     ConfigMap::new(MapMatch::False, 0),
///     ConfigMap::new(MapMatch::True, 1),
///     ConfigMap::new(MapMatch::String(c"input"), 2),
/// ];
/// assert_eq!(AUTOCRLF[2].as_ref().rule(), MapMatch::String(c"input"));
/// ```
///
/// Records cross threads, but only with a string that outlives the thread:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::config::config::{ConfigMap, MapMatch};
/// use std::ffi::CString;
/// let text = CString::new("temporary").unwrap();
/// let map = ConfigMap::new(MapMatch::String(&text), 1);
/// std::thread::spawn(move || drop(map));
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::config::config::{ConfigMap, MapMatch};
/// use std::ffi::CString;
/// let map;
/// { let text = CString::new("temporary").unwrap();
///   map = ConfigMap::new(MapMatch::String(&text), 1); }
/// assert!(map.as_ref().str_match().is_some());
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::config::config::{ConfigMap, MapMatch};
/// let mut map = ConfigMap::new(MapMatch::String(c"one"), 1);
/// let mut m = map.as_mut();
/// let view = m.as_ref().str_match();
/// m.set_match(MapMatch::String(c"two"));
/// assert!(view.is_some());
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::config::config::ConfigMapMut;
/// fn shrink<'a, 's>(m: ConfigMapMut<'a, 'static>) -> ConfigMapMut<'a, 's> { m }
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::config::config::{ConfigMap, MapMatch};
/// let map = ConfigMap::new(MapMatch::String(c"live"), 1);
/// let text = map.as_ref().str_match();
/// drop(map);
/// assert!(text.is_some());
/// ```
///
/// Wraps: `git_configmap`
/// Handwritten
#[repr(transparent)]
pub struct ConfigMap<'s>(ffi::git_configmap, PhantomData<(*const (), &'s CStr)>);

/// Shared mapping borrow; string views cannot outlive it.
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ConfigMapRef<'a, 's>(CBorrowedPtr<'a, ConfigMap<'s>>);

/// Exclusive mapping borrow, invariant in the stored-string lifetime.
/// Handwritten
#[repr(transparent)]
pub struct ConfigMapMut<'a, 's>(ConfigMapRef<'a, 's>, PhantomData<&'a mut ConfigMap<'s>>);

// SAFETY: transparent layout over git_configmap; transparent, pointer-wide
// handles with no Drop; Mut is invariant in Self and references cover only
// Rust-owned inline storage, never a borrowed C record.
unsafe impl<'s> CCell for ConfigMap<'s> {
    type C = ffi::git_configmap;
    type Mut<'a>
        = ConfigMapMut<'a, 's>
    where
        Self: 'a;
    type Ref<'a>
        = ConfigMapRef<'a, 's>
    where
        Self: 'a;
}

// SAFETY: the record is two scalars and a pointer that only the setters write,
// NULL or a `&'s CStr`'s: a shared, immutable borrow of `CStr`, which is `Sync`.
// Nothing has thread affinity or interior mutability, and nothing is released on
// drop, so moving the record moves bytes.
unsafe impl Send for ConfigMap<'_> {}
// SAFETY: as above; `ConfigMapMut` is an exclusive borrow, as `&mut`.
unsafe impl Send for ConfigMapMut<'_, '_> {}

// SAFETY: as above, and shared access only reads the fields (every write goes
// through `ConfigMapMut`, which excludes other handles), so readers race nothing.
unsafe impl Sync for ConfigMap<'_> {}
// SAFETY: as above; `ConfigMapRef` is a shared borrow, as `&`.
unsafe impl Send for ConfigMapRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for ConfigMapRef<'_, '_> {}
// SAFETY: `&ConfigMapMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for ConfigMapMut<'_, '_> {}

impl<'s> ConfigMap<'s> {
    /// Builds a resource-free record with a valid discriminator/string pair.
    #[must_use]
    pub const fn new(rule: MapMatch<'s>, map_value: i32) -> Self {
        Self(
            ffi::git_configmap {
                type_: rule.map_type().raw(),
                str_match: match rule {
                    MapMatch::String(s) => s.as_ptr(),
                    _ => ptr::null(),
                },
                map_value,
            },
            PhantomData,
        )
    }

    /// A shared handle over the same object, borrowed from this one.
    #[must_use]
    pub fn as_ref(&self) -> ConfigMapRef<'_, 's> {
        // SAFETY: initialized Rust-owned record shared for the returned lifetime.
        ConfigMapRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// An exclusive handle over the same object, reborrowed from this one.
    pub fn as_mut(&mut self) -> ConfigMapMut<'_, 's> {
        // SAFETY: exclusive Rust borrow supplies live storage and write provenance.
        let p = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        ConfigMapMut(ConfigMapRef(p), PhantomData)
    }
}

impl Clone for ConfigMap<'_> {
    fn clone(&self) -> Self {
        Self(self.0, PhantomData)
    }
}

impl fmt::Debug for ConfigMap<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for ConfigMapRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigMap")
            .field("map_type", &self.map_type())
            .field("str_match", &self.str_match())
            .field("map_value", &self.map_value())
            .finish()
    }
}

impl fmt::Debug for ConfigMapMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 's> ConfigMapRef<'a, 's> {
    /// # Safety
    /// NULL or a live aligned initialized record, unchanged for `'a`, with
    /// a known mode and immutable NUL string valid for `'s` when non-NULL;
    /// String mode requires non-NULL. `'s` must outlive `'a`.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) unsafe fn from_ptr(raw: *const ffi::git_configmap) -> Option<Self> {
        let p = NonNull::new(raw.cast_mut().cast::<ConfigMap<'s>>())?;
        // SAFETY: caller proves layout, validity, liveness and string lifetimes.
        Some(Self(unsafe { CBorrowedPtr::new(p) }))
    }

    pub(crate) fn as_ptr(&self) -> *const ffi::git_configmap {
        self.0.as_non_null().as_ptr().cast()
    }

    /// Field: `git_configmap.type`
    ///
    /// # Panics
    ///
    /// Panics, with `invalid mapping discriminator`, only if libgit2 breaks its contract.
    #[must_use]
    pub fn map_type(&self) -> MapType {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let type_ptr = unsafe { &raw const (*self.as_ptr()).type_ };
        // SAFETY: live shared record; constructors/seams ensure a known mode.
        let raw = unsafe { type_ptr.read() };
        MapType::from_raw(raw).expect("invalid mapping discriminator")
    }

    /// Field: `git_configmap.map_value`
    #[must_use]
    pub fn map_value(&self) -> i32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let map_value_ptr = unsafe { &raw const (*self.as_ptr()).map_value };
        // SAFETY: reads an initialized scalar through the live shared handle.
        unsafe { map_value_ptr.read() }
    }

    /// None for ordinary non-string rules; String mode always has a string.
    /// Raw C non-string records may also have an ignored string, preserved here.
    ///
    /// Field: `git_configmap.str_match`
    #[must_use]
    pub fn str_match(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let str_match_ptr = unsafe { &raw const (*self.as_ptr()).str_match };
        // SAFETY: the shared live handle permits reading the initialized pointer.
        let p = unsafe { str_match_ptr.read() };
        if p.is_null() {
            None
        } else {
            // SAFETY: the record's immutable NUL string lives for 's, which
            // outlives the handle's 'a; constructors and seams preserve this.
            Some(unsafe { CStr::from_ptr(p) })
        }
    }

    /// The rule this record describes: its mapping type, with its string for a string rule.
    #[must_use]
    #[expect(clippy::missing_panics_doc, reason = "cannot panic: NULL string rule")]
    pub fn rule(&self) -> MapMatch<'a> {
        match self.map_type() {
            MapType::False => MapMatch::False,
            MapType::True => MapMatch::True,
            MapType::Int32 => MapMatch::Int32,
            MapType::String => MapMatch::String(self.str_match().expect("NULL string rule")),
        }
    }
}

impl<'s> ConfigMapMut<'_, 's> {
    /// A shared handle over the same object, borrowed from this one.
    #[must_use]
    pub fn as_ref(&self) -> ConfigMapRef<'_, 's> {
        self.0
    }

    /// An exclusive handle over the same object, reborrowed from this one.
    pub fn as_mut(&mut self) -> ConfigMapMut<'_, 's> {
        ConfigMapMut(self.0, PhantomData)
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_configmap {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Replaces mode and string together, preserving String's non-NULL invariant.
    pub fn set_match(&mut self, rule: MapMatch<'s>) {
        let p = match rule {
            MapMatch::String(s) => s.as_ptr(),
            _ => ptr::null(),
        };
        // SAFETY: exclusive live record, with no outstanding shared view;
        // the new referent is borrowed for 's. Neither referent is owned here.
        let value2 = unsafe { addr_of_mut!((*self.as_mut_ptr()).str_match) };
        // SAFETY: as above.
        unsafe { value2.write(p) };
        // SAFETY: as above.
        let value2 = unsafe { addr_of_mut!((*self.as_mut_ptr()).type_) };
        // SAFETY: as above.
        unsafe { value2.write(rule.map_type().raw()) };
    }

    /// Replaces the string borrow without changing the mode. Non-string modes
    /// may store an ignored string too; `set_match` clears it for those modes.
    /// A non-null argument preserves String mode's required string invariant.
    pub fn set_str_match(&mut self, value: &'s CStr) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let str_match_ptr = unsafe { &raw mut (*self.as_mut_ptr()).str_match };
        // SAFETY: exclusive initialized record; the replacement immutable NUL
        // string lives for 's. Neither the previous nor new referent is owned.
        unsafe { str_match_ptr.write(value.as_ptr()) };
    }

    /// Sets the value the rule maps a match to.
    pub fn set_map_value(&mut self, value: i32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let map_value_ptr = unsafe { &raw mut (*self.as_mut_ptr()).map_value };
        // SAFETY: exclusive live handle allows replacing the initialized scalar.
        unsafe { map_value_ptr.write(value) };
    }
}

#[cfg(test)]
pub(crate) mod unit_tests {
    use core::ffi::{CStr, c_int, c_void};
    use core::mem::size_of;
    use std::process::Command;
    use std::sync::atomic::Ordering::SeqCst;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    use super::*;
    use crate::libgit2::util::errors::ErrorCode;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};

    /// Rust-only fixture for the borrowed callback seam; never adopted as an
    /// owner, since it lacks the enclosing C backend-entry release callback.
    fn callback_entry(raw: &mut ffi::git_config_entry) -> EntryRef<'_> {
        // SAFETY: this initialized Rust fixture has only NULL/static strings
        // and stays live and unchanged for the exclusive borrow's lifetime.
        unsafe { EntryRef::from_ptr(ptr::from_mut(raw)) }.unwrap()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn enumeration_initial_allocation_failure_is_reported() {
        isolated(
            concat!(
                module_path!(),
                "::enumeration_initial_allocation_failure_is_reported"
            ),
            || {
                install_refusing();
                let config = Config::new().unwrap();
                // Allocate the thread's error state before refusing C allocations.
                assert_eq!(get(config.as_ref(), c"a.b"), Err(ErrorCode::NotFound.raw()));
                let mut f = |_: EntryRef<'_>| ControlFlow::Continue(());
                let mut cb = ForeachCallback::new(&mut f);
                REFUSING.store(true, SeqCst);
                let results = [
                    config.as_ref().foreach(&mut cb),
                    config.as_ref().foreach_match(Some(c".*"), &mut cb),
                    config.as_ref().foreach_multivar(c"a.b", None, &mut cb),
                    config
                        .as_ref()
                        .foreach_multivar(c"a.b", Some(c"."), &mut cb),
                ];
                REFUSING.store(false, SeqCst);
                for result in results {
                    assert_eq!(result.unwrap_err().class(), Some(ErrorClass::NoMemory));
                }
                assert_eq!(cb.stopped(), None);
                assert_eq!(
                    config.as_ref().foreach(&mut cb).unwrap(),
                    ControlFlow::Continue(())
                );
            },
        );
    }

    #[test]
    fn enumeration_result_preserves_stops_even_when_c_erases_iterover() {
        let mut f = |_: EntryRef<'_>| ControlFlow::Continue(());
        let mut callback = ForeachCallback::new(&mut f);
        assert_eq!(
            foreach_result(0, &callback).unwrap(),
            ControlFlow::Continue(())
        );
        for raw in [
            ffi::GIT_ITEROVER,
            ffi::GIT_EUSER,
            -1,
            1,
            c_int::MIN,
            c_int::MAX,
        ] {
            let code = NonZero::new(raw).unwrap();
            callback.stopped = Some(code);
            assert_eq!(
                foreach_result(0, &callback).unwrap(),
                ControlFlow::Break(code)
            );
            assert_eq!(
                foreach_result(raw, &callback).unwrap(),
                ControlFlow::Break(code)
            );
        }
    }

    #[test]
    fn callback_trampoline_stops_panics_nulls_and_reuses() {
        let mut raw = ffi::git_config_entry {
            name: c"a.b".as_ptr(),
            value: c"input".as_ptr(),
            backend_type: ptr::null(),
            origin_path: ptr::null(),
            include_depth: u32::MAX,
            level: i32::MAX,
        };
        let entry = callback_entry(&mut raw);
        let mut count = 0;
        let mut f = |e: EntryRef<'_>| {
            count += 1;
            assert_eq!(e.value(), Some(c"input"));
            assert_eq!(e.include_depth(), u32::MAX);
            assert!(count != 1, "caught inside trampoline");
            ControlFlow::Break(NonZero::new(73).unwrap())
        };
        let mut callback = ForeachCallback::new(&mut f);
        // A hostile internal invoker ignoring a stop cannot invoke the closure
        // again, overwrite its panic, or drop a panic payload inside C.
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            callback.walk_with(|cb, payload| {
                // SAFETY: live exclusively borrowed handle, immutable fixture entry,
                // no concurrent/reentrant call or retained pointer.
                let invoke = || unsafe { cb.unwrap()(entry.as_ptr(), payload) };
                assert_eq!(invoke(), ErrorCode::User.raw());
                assert_eq!(invoke(), ErrorCode::User.raw());
            });
        }));
        assert!(caught.is_err());
        assert_eq!(
            callback.call(entry),
            ControlFlow::Break(NonZero::new(73).unwrap())
        );
        assert!(format!("{callback:?}").contains("73"));
        let rc = callback.walk_with(|cb, payload| {
            // SAFETY: valid exclusive callback payload; NULL entries are rejected.
            unsafe { cb.unwrap()(ptr::null(), payload) }
        });
        assert_eq!(rc, ErrorCode::User.raw());
        drop(callback);
        assert_eq!(count, 2);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn callback_foreign_iteration_cleans_up_before_resuming_panic() {
        let file =
            std::env::temp_dir().join(format!("crustify-callback-w2b1-{}", std::process::id()));
        std::fs::write(&file, b"[a]\n b = 1\n c = 2\n").unwrap();
        let config = Config::open(&CString::new(file.to_str().unwrap()).unwrap()).unwrap();
        let mut count = 0;
        let mut f = |entry: EntryRef<'_>| {
            assert!(entry.name().is_some());
            count += 1;
            assert!(count != 1, "foreign iteration panic");
            ControlFlow::Continue(())
        };
        let mut callback = ForeachCallback::new(&mut f);
        let walk = |callback: &mut ForeachCallback<'_>| config.as_ref().foreach(callback);
        assert!(panic::catch_unwind(AssertUnwindSafe(|| walk(&mut callback))).is_err());
        assert_eq!(walk(&mut callback).unwrap(), ControlFlow::Continue(()));
        assert_eq!(callback.stopped(), None);
        drop(callback);
        assert_eq!(count, 3);
        drop(config);
        std::fs::remove_file(file).unwrap();
    }

    /// Nothing but a boolean-true rule maps a valueless setting, and C reports the
    /// miss by formatting its NULL value through `%s`. The wrapper reports the miss
    /// itself, worded as glibc renders C's, and still leaves tables with a true rule
    /// to C, whose speculative integer parse error survives the match.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn valueless_misses_never_reach_c_formatting() {
        let missing = [
            ConfigMap::new(MapMatch::Int32, 1),
            ConfigMap::new(MapMatch::False, 2),
            ConfigMap::new(MapMatch::String(c"x"), 3),
        ];
        let with_true = [
            ConfigMap::new(MapMatch::Int32, 1),
            ConfigMap::new(MapMatch::True, 5),
        ];
        let file =
            std::env::temp_dir().join(format!("crustify-valueless-map-{}", std::process::id()));
        std::fs::write(&file, b"[a]\n flag\n").unwrap();
        let config = Config::open(&CString::new(file.to_str().unwrap()).unwrap()).unwrap();
        for maps in [&[][..], &missing[..1], &missing[..]] {
            set(ErrorClass::Odb, c"previous");
            let lookup = lookup_map_value(maps, None).unwrap_err();
            set(ErrorClass::Odb, c"previous");
            let mapped = config.as_ref().get_mapped(c"a.flag", maps).unwrap_err();
            for error in [lookup, mapped] {
                assert_eq!(error.raw_code(), ffi::GIT_ERROR);
                assert_eq!(error.class(), Some(ErrorClass::Config));
                assert_eq!(error.message(), c"failed to map '(null)'");
            }
            assert_eq!(last().message(), c"failed to map '(null)'");
        }
        clear();
        assert_eq!(lookup_map_value(&with_true, None).unwrap(), 5);
        let parse_error = c"failed to parse '(null)' as a 32-bit integer";
        assert_eq!(last().message(), parse_error);
        clear();
        assert_eq!(
            config.as_ref().get_mapped(c"a.flag", &with_true).unwrap(),
            5
        );
        assert_eq!(last().message(), parse_error);
        clear();
        drop(config);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mapped_snapshot_construction_failure_reports_oom() {
        isolated(
            concat!(
                module_path!(),
                "::mapped_snapshot_construction_failure_reports_oom"
            ),
            || {
                install_refusing();
                let config = Config::new().unwrap();
                // Warm the thread's error state before limiting C allocations.
                assert_eq!(
                    config.as_ref().get_i32(c"a.b").unwrap_err().code(),
                    Some(ErrorCode::NotFound)
                );
                // The empty multivar iterator and normalized key allocate three
                // times; the next allocation, the empty snapshot, fails. No file
                // entry copying (whose C OOM paths are defective) is involved.
                ALLOWED_ALLOCATIONS.store(3, SeqCst);
                let result = config.as_ref().get_mapped(c"a.b", &[]);
                ALLOWED_ALLOCATIONS.store(usize::MAX, SeqCst);
                assert_eq!(result.unwrap_err().class(), Some(ErrorClass::NoMemory));
                assert_eq!(
                    config.as_ref().get_mapped(c"a.b", &[]).unwrap_err().code(),
                    Some(ErrorCode::NotFound)
                );
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn file_priority_rejected_before_foreign_comparators() {
        let mut cfg = Config::new().unwrap();
        for value in [i32::MIN, -2, -1, i32::MAX] {
            let error = cfg
                .as_mut()
                .add_file(c"unused", ConfigLevel::from_raw(value), None, false)
                .unwrap_err();
            assert_eq!(error.raw_code(), ffi::GIT_EINVALID);
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn get_i64_silent_parse_failure_is_not_a_stale_error() {
        let file =
            std::env::temp_dir().join(format!("crustify-get-i64-silent-{}", std::process::id()));
        std::fs::write(&file, b"[a]\n number = 1kb\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let cfg = Config::open(&path).unwrap();
        set(ErrorClass::Invalid, c"unrelated earlier failure");
        let error = cfg.as_ref().get_i64(c"a.number").unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Config));
        assert_eq!(
            error.message(),
            c"failed to parse configuration value as an integer"
        );
        drop(cfg);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn mapping_layout_borrows_clone_and_seams() {
        assert_eq!(size_of::<ConfigMap<'_>>(), size_of::<ffi::git_configmap>());
        assert_eq!(
            align_of::<ConfigMap<'_>>(),
            align_of::<ffi::git_configmap>()
        );
        assert_eq!(size_of::<ConfigMapRef<'_, '_>>(), size_of::<*const ()>());
        assert_eq!(size_of::<ConfigMapMut<'_, '_>>(), size_of::<*mut ()>());
        let text = CString::new([0xFF, b'x']).unwrap();
        let mut map = ConfigMap::new(MapMatch::String(&text), i32::MIN);
        let cloned = map.clone();
        let mut m = map.as_mut();
        m.as_mut().set_map_value(i32::MAX);
        assert_eq!(m.as_ref().map_value(), i32::MAX);
        for rule in [
            MapMatch::False,
            MapMatch::True,
            MapMatch::Int32,
            MapMatch::String(c""),
        ] {
            m.set_match(rule);
            assert_eq!(m.as_ref().rule(), rule);
            // SAFETY: the exclusive handle is frozen during this shared reborrow;
            // initialized record and its string remain live and immutable.
            let shared = unsafe { ConfigMapRef::from_ptr(m.as_ref().as_ptr()) }.unwrap();
            assert_eq!(shared.rule(), rule);
            m.set_str_match(&text);
            assert_eq!(m.as_ref().str_match(), Some(text.as_c_str()));
        }
        assert_eq!(cloned.as_ref().str_match(), Some(text.as_c_str()));
        // SAFETY: NULL is refused without forming a handle.
        assert!(unsafe { ConfigMapRef::from_ptr(ptr::null()) }.is_none());
    }

    #[test]
    fn entry_layout_and_borrowed_fields_without_c() {
        assert_eq!(size_of::<Entry>(), size_of::<ffi::git_config_entry>());
        assert_eq!(align_of::<Entry>(), align_of::<ffi::git_config_entry>());
        assert_eq!(size_of::<EntryRef<'_>>(), size_of::<*const ()>());
        assert_eq!(size_of::<EntryMut<'_>>(), size_of::<*mut ()>());
        assert_eq!(size_of::<EntryArc>(), size_of::<*mut ()>());
        assert_eq!(size_of::<Option<EntryArc>>(), size_of::<*mut ()>());
        let mut raw = ffi::git_config_entry {
            name: c"a.b".as_ptr(),
            value: c"\xff".as_ptr(),
            backend_type: c"".as_ptr(),
            origin_path: ptr::null(),
            include_depth: u32::MAX,
            level: i32::MAX,
        };
        // SAFETY: this Rust-owned fixture has initialized fields and static NUL
        // strings, and remains unchanged while the shared borrow lives. It is
        // not a backend-owned entry and must never be adopted by EntryFree.
        let entry = unsafe { EntryRef::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(entry.name(), Some(c"a.b"));
        assert_eq!(entry.value(), Some(c"\xff"));
        assert_eq!(entry.backend_type(), Some(c""));
        assert_eq!(entry.origin_path(), None);
        assert_eq!(entry.level(), ConfigLevel::from_raw(i32::MAX));
        assert_eq!(entry.include_depth(), u32::MAX);
        raw.name = ptr::null();
        raw.value = ptr::null();
        raw.backend_type = ptr::null();
        raw.origin_path = c"origin".as_ptr();
        // SAFETY: as above, after the earlier handle's last use.
        let entry = unsafe { EntryRef::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(entry.name(), None);
        assert_eq!(entry.value(), None);
        assert_eq!(entry.backend_type(), None);
        assert_eq!(entry.origin_path(), Some(c"origin"));
        // SAFETY: NULL is refused before constructing a handle.
        assert!(unsafe { EntryRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: no shared handle remains in use; this Rust-owned fixture is
        // exclusively borrowed and strings are static. No ownership is adopted.
        let mut exclusive = unsafe { EntryMut::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(exclusive.as_ref().origin_path(), Some(c"origin"));
        assert_eq!(exclusive.as_mut().as_ref().value(), None);
    }

    /// Mapping records and their handles cross threads; entry handles print
    /// their fields, and mapping records print their rule.
    #[test]
    fn debug_and_thread_traits() {
        fn shared<T: Send + Sync>(_: &T) {}
        const TABLE: [ConfigMap<'static>; 2] = [
            ConfigMap::new(MapMatch::Int32, 0),
            ConfigMap::new(MapMatch::String(c"in"), 2),
        ];
        let mut map = TABLE[1].clone();
        shared(&map);
        shared(&map.as_ref());
        shared(&map.as_mut());
        let expected = r#"ConfigMap { map_type: String, str_match: Some("in"), map_value: 2 }"#;
        assert_eq!(format!("{map:?}"), expected);
        assert_eq!(format!("{:?}", map.as_ref()), expected);
        assert_eq!(format!("{:?}", map.as_mut()), expected);
        assert_eq!(
            format!("{:?}", TABLE[0]),
            "ConfigMap { map_type: Int32, str_match: None, map_value: 0 }"
        );

        let mut raw = ffi::git_config_entry {
            name: c"a.b".as_ptr(),
            value: ptr::null(),
            backend_type: c"file".as_ptr(),
            origin_path: c"\xff".as_ptr(),
            include_depth: 1,
            level: ffi::GIT_CONFIG_LEVEL_LOCAL,
        };
        let expected = concat!(
            r#"Entry { name: Some("a.b"), value: None, backend_type: Some("file"), "#,
            r#"origin_path: Some("\xff"), level: ConfigLevel(5), include_depth: 1 }"#
        );
        // SAFETY: a Rust-owned fixture with initialized fields and static NUL
        // strings, unchanged while the handle lives; nothing adopts it.
        let entry = unsafe { EntryRef::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(format!("{entry:?}"), expected);
        // SAFETY: as above, after the shared handle's last use; exclusive.
        let exclusive = unsafe { EntryMut::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(format!("{exclusive:?}"), expected);
        let value = Entry(raw, PhantomData);
        assert_eq!(format!("{value:?}"), expected);
    }

    /// A backend that cannot reread its changed file fails a lookup that would
    /// consult it, with libgit2's parse error, and without the unwritten entry
    /// pointer C's `get_entry` forms for it (`UBSan`: member access within null
    /// pointer). Afterwards the backend serves its old entries, as in C. An
    /// invalid name fails first, without spending the reread; a failure spent
    /// below the key's backend leaves the thread's error as it was.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entry_reports_failed_rereads() {
        let dir =
            std::env::temp_dir().join(format!("crustify-entry-rereads-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        let (low, high) = (dir.join("global"), dir.join("local"));
        std::fs::write(&low, b"[test]\n shared = g\n").unwrap();
        std::fs::write(&high, b"[test]\n key = high\n").unwrap();
        let mut cfg = Config::new().unwrap();
        for (path, level) in [(&low, ConfigLevel::GLOBAL), (&high, ConfigLevel::LOCAL)] {
            let path = CString::new(path.to_str().unwrap()).unwrap();
            cfg.as_mut().add_file(&path, level, None, false).unwrap();
        }
        let value = |cfg: &ConfigBox<'_>, key: &CStr| {
            let entry = cfg.as_ref().entry(key)?;
            Ok::<_, ConfigError>(entry.as_ref().value().unwrap().to_owned())
        };
        let parse_error = |path: &std::path::Path| {
            format!(
                "failed to parse config file: missing ']' in section header (in {}:1)",
                path.display()
            )
        };

        std::fs::write(&high, b"[test\n key = broken\n").unwrap();
        let error = value(&cfg, c"test.key").unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Generic));
        assert_eq!(error.class(), Some(ErrorClass::Config));
        assert_eq!(error.message().to_str().unwrap(), parse_error(&high));
        assert_eq!(last(), *error.error());
        assert_eq!(value(&cfg, c"test.key").unwrap().as_c_str(), c"high");
        assert_eq!(value(&cfg, c"test.shared").unwrap().as_c_str(), c"g");

        std::fs::write(&high, b"[test\n key = broken again\n").unwrap();
        let error = value(&cfg, c"bad").unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::InvalidSpec));
        assert_eq!(error.message(), c"invalid config item name 'bad'");
        let error = value(&cfg, c"test.shared").unwrap_err();
        assert_eq!(error.message().to_str().unwrap(), parse_error(&high));

        for seeded in [true, false] {
            std::fs::write(
                &low,
                if seeded {
                    &b"[test\n"[..]
                } else {
                    &b"[test\n\n"[..]
                },
            )
            .unwrap();
            if seeded {
                set(ErrorClass::Odb, c"an earlier, unrelated failure");
            } else {
                clear();
            }
            let before = last();
            assert_eq!(value(&cfg, c"test.key").unwrap().as_c_str(), c"high");
            assert_eq!(last(), before, "seeded: {seeded}");
        }
        assert_eq!(value(&cfg, c"test.shared").unwrap().as_c_str(), c"g");
        drop(cfg);
        std::fs::remove_dir_all(dir).unwrap();
    }

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
    pub(crate) fn isolated(test: &str, body: fn()) {
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

    #[test]
    #[cfg_attr(miri, ignore = "error reporting calls libgit2")]
    fn write_order_length_is_checked_before_ffi() {
        for len in [0, 1, c_int::MAX as usize - 1] {
            assert!(validate_write_order_len(len).is_ok());
        }
        for len in [c_int::MAX as usize, c_int::MAX as usize + 1, usize::MAX] {
            let error = validate_write_order_len(len).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Generic));
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(
                error.message(),
                c"write order length must be less than INT_MAX"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn snapshot_allocation_failure_leaves_source_live() {
        isolated(
            concat!(
                module_path!(),
                "::snapshot_allocation_failure_leaves_source_live"
            ),
            || {
                install_refusing();
                let source = Config::new().unwrap();
                assert_eq!(get(source.as_ref(), c"a.b"), Err(ErrorCode::NotFound.raw()));
                REFUSING.store(true, SeqCst);
                let error = source.as_ref().snapshot().unwrap_err();
                REFUSING.store(false, SeqCst);
                assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                assert_eq!(get(source.as_ref(), c"a.b"), Err(ErrorCode::NotFound.raw()));
                let snapshot = source.as_ref().snapshot().unwrap();
                drop(source);
                assert_eq!(
                    get(snapshot.as_ref(), c"a.b"),
                    Err(ErrorCode::NotFound.raw())
                );
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn snapshot_backend_allocation_failure_releases_new_config() {
        isolated(
            concat!(
                module_path!(),
                "::snapshot_backend_allocation_failure_releases_new_config"
            ),
            || {
                install_refusing();
                let mut source = Config::new().unwrap();
                let path = std::env::temp_dir()
                    .join(format!("crustify-snapshot-oom-{}", std::process::id()));
                std::fs::write(&path, "[a]\n b = 12\n").unwrap();
                let file = CString::new(path.to_str().unwrap()).unwrap();
                source
                    .as_mut()
                    .add_file(&file, ConfigLevel::LOCAL, None, false)
                    .unwrap();
                assert_eq!(get(source.as_ref(), c"a.b"), Ok(12));
                // Prime TLS error storage before refusing allocations. The fresh
                // output config and its two vectors need three allocations; the
                // next (the backend snapshot) fails. Partial ownership stays C's.
                assert_eq!(
                    get(source.as_ref(), c"a.missing"),
                    Err(ErrorCode::NotFound.raw())
                );
                ALLOWED_ALLOCATIONS.store(3, SeqCst);
                let error = source.as_ref().snapshot().unwrap_err();
                ALLOWED_ALLOCATIONS.store(usize::MAX, SeqCst);
                assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                assert_eq!(get(source.as_ref(), c"a.b"), Ok(12));
                let snapshot = source.as_ref().snapshot().unwrap();
                drop(source);
                assert_eq!(get(snapshot.as_ref(), c"a.b"), Ok(12));
                drop(snapshot);
                std::fs::remove_file(path).unwrap();
            },
        );
    }

    /// Refusing each allocation of an entry lookup in turn, through the reread
    /// walk and the snapshot, fails with libgit2's out-of-memory error, leaks
    /// nothing (`LSan`) and leaves the config usable, until the lookup completes.
    /// The config has no file backend: a file backend's iterator and snapshot
    /// duplicate its entry list, and `git_config_list_dup_entry` corrupts the
    /// heap when an allocation fails there
    /// (`reviews/.../config/wave-1/batch-0/misc/
    /// config-list-dup-corrupts-heap-on-allocation-failure`). The lookup's second allocation is
    /// never refused: C leaks the iterator built by the first
    /// (`misc/multivar-iterator-leaks-inner-on-allocation-failure`).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn entry_allocation_failures_leave_config_usable() {
        isolated(
            concat!(
                module_path!(),
                "::entry_allocation_failures_leave_config_usable"
            ),
            || {
                install_refusing();
                let cfg = Config::new().unwrap();
                // Prime TLS error storage before refusing allocations.
                let missing = cfg.as_ref().entry(c"a.b").unwrap_err();
                assert_eq!(missing.code(), Some(ErrorCode::NotFound));
                let mut granted = 0;
                let error = loop {
                    assert!(granted < 10_000, "lookup never completed");
                    if granted == 1 {
                        granted += 1;
                        continue;
                    }
                    ALLOWED_ALLOCATIONS.store(granted, SeqCst);
                    let error = cfg.as_ref().entry(c"a.b").unwrap_err();
                    ALLOWED_ALLOCATIONS.store(usize::MAX, SeqCst);
                    if error.class() != Some(ErrorClass::NoMemory) {
                        break error;
                    }
                    assert_eq!(
                        cfg.as_ref().entry(c"a.c").unwrap_err().code(),
                        missing.code()
                    );
                    granted += 1;
                };
                assert!(granted > 1, "both stages allocate");
                assert_eq!(error, missing);
            },
        );
    }

    static REFUSING: AtomicBool = AtomicBool::new(false);
    pub(crate) static ALLOWED_ALLOCATIONS: AtomicUsize = AtomicUsize::new(usize::MAX);

    fn refuse_allocation() -> bool {
        if REFUSING.load(SeqCst) {
            return true;
        }
        let mut remaining = ALLOWED_ALLOCATIONS.load(SeqCst);
        loop {
            if remaining == usize::MAX {
                return false;
            }
            if remaining == 0 {
                return true;
            }
            match ALLOWED_ALLOCATIONS.compare_exchange(remaining, remaining - 1, SeqCst, SeqCst) {
                Ok(_) => return false,
                Err(current) => remaining = current,
            }
        }
    }

    // libc's allocator, declared with its C signatures.
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
            if refuse_allocation() {
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
            if refuse_allocation() {
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

    pub(crate) fn install_refusing() {
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

    /// `git_config_get_int32` on `config`: the value, or the return code.
    fn get(config: ConfigRef<'_, '_>, name: &CStr) -> Result<i32, c_int> {
        config.get_i32(name).map_err(|e| e.raw_code())
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(
            size_of::<ConfigBox<'_>>(),
            size_of::<*mut ffi::git_config>()
        );
        assert_eq!(
            size_of::<ConfigArc<'_>>(),
            size_of::<*mut ffi::git_config>()
        );
        assert_eq!(
            size_of::<Option<ConfigArc<'_>>>(),
            size_of::<*mut ffi::git_config>()
        );
        assert_eq!(
            size_of::<ConfigRef<'_, '_>>(),
            size_of::<*const ffi::git_config>()
        );
        assert_eq!(
            size_of::<ConfigMut<'_, '_>>(),
            size_of::<*const ffi::git_config>()
        );
        assert_eq!(size_of::<ConfigFree>(), 0);
    }

    /// The handles wrap a pointer without reading through it: `from_ptr` refuses NULL
    /// and keeps any other pointer, `as_ptr` gives it back, and `Debug` prints it.
    #[test]
    fn handles_wrap_pointers_without_reading() {
        let dangling: *mut ffi::git_config = NonNull::dangling().as_ptr();
        // SAFETY: the handle is never dereferenced: `as_ptr` and `Debug` only read the
        // pointer itself, which is what this test checks.
        let handle = unsafe { ConfigRef::<'_, 'static>::from_ptr(dangling) }.unwrap();
        assert_eq!(handle.as_ptr(), dangling);
        assert_eq!(format!("{handle:?}"), format!("ConfigRef({dangling:p})"));
        // SAFETY: NULL is refused before any handle exists.
        assert!(unsafe { ConfigRef::<'_, 'static>::from_ptr(ptr::null_mut()) }.is_none());
    }

    /// The exclusive handle reborrows shared and exclusive, and prints as the shared
    /// one.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn exclusive_handle_reborrows() {
        let mut sole = Config::new().unwrap();
        let raw = sole.as_c_ptr();
        let mut m = sole.as_mut();
        assert_eq!(m.as_mut().as_mut_ptr(), raw);
        assert_eq!(m.as_mut().as_ref().as_ptr(), raw);
        assert_eq!(m.as_ref().as_ptr(), raw);
        assert_eq!(format!("{m:?}"), format!("ConfigRef({raw:p})"));
        assert_eq!(
            get(m.as_ref(), c"core.bare"),
            Err(ErrorCode::NotFound.raw())
        );
    }

    /// Every construction path builds an empty config, which misses every lookup and
    /// refuses writes, and is freed once (`LSan` flags a leak, `ASan` a double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_build_empty_configs() {
        let mut sole = Config::new().unwrap();
        assert_eq!(
            get(sole.as_ref(), c"core.bare"),
            Err(ErrorCode::NotFound.raw())
        );
        let mut m = sole.as_mut();
        // SAFETY: `m` is the only handle to a live config, written for the call; the
        // name and value are NUL-terminated.
        let rc = unsafe { ffi::git_config_set_int32(m.as_mut_ptr(), c"core.x".as_ptr(), 1) };
        assert!(rc < 0, "no backend to write to");
        assert_eq!(last().class(), Some(ErrorClass::Config));

        let through_policy = ConfigBox::new().unwrap();
        let shared = ConfigArc::new().unwrap();
        let other = ConfigArc::from(through_policy);
        assert!(!ConfigArc::ptr_eq(&shared, &other));
        assert_eq!(
            get(shared.as_ref(), c"core.bare"),
            Err(ErrorCode::NotFound.raw())
        );
        assert_eq!(format!("{other:p}"), format!("{:p}", other.as_c_ptr()));
    }

    /// A repository's config, adopted as a shared reference of the repository's borrow,
    /// is shared with the repository and released once per reference (`ASan` flags a use
    /// after free or a double free, `LSan` a leak); the repository goes last.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn repository_config_is_shared_for_the_repository_borrow() {
        assert!(runtime_pinned());
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/c/tests/resources/testrepo.git\0"
        );
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` receives the repository; `path` is NUL-terminated; no ceiling
        // directories.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &raw mut repo,
                path.as_ptr().cast(),
                ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                ptr::null(),
            )
        };
        assert_eq!(rc, 0);
        let adopt = || {
            let mut out = ptr::null_mut();
            // SAFETY: `out` is a writable slot, `repo` open.
            assert_eq!(unsafe { ffi::git_repository_config(&raw mut out, repo) }, 0);
            // SAFETY: `git_repository_config` hands out one up-ref'd reference to the
            // repository's config, released by `git_config_free`; the repository holds
            // another, so it is shared, and outlives the arcs (freed below, after them).
            unsafe { ConfigArc::<'_>::from_c(out) }.unwrap()
        };
        let (first, second) = (adopt(), adopt());
        assert!(ConfigArc::ptr_eq(&first, &second));
        assert_eq!(get(first.as_ref(), c"core.repositoryformatversion"), Ok(0));
        drop(first);
        assert_eq!(get(second.as_ref(), c"core.repositoryformatversion"), Ok(0));
        assert_eq!(
            get(second.as_ref(), c"core.missing"),
            Err(ErrorCode::NotFound.raw())
        );
        drop(second);
        // SAFETY: `repo` is open and freed once, after every reference to its config.
        unsafe { ffi::git_repository_free(repo) };
    }

    /// An allocation failure is `None` from the policy and libgit2's out-of-memory
    /// error from `Config::new`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_memory_is_an_error() {
        isolated(
            concat!(module_path!(), "::out_of_memory_is_an_error"),
            || {
                install_refusing();
                assert!(runtime_pinned());
                let kept = Config::new().unwrap();
                // A first error allocates the thread's error state, which an
                // allocation failure would otherwise report instead (`GIT_ERROR_THREAD`).
                assert_eq!(get(kept.as_ref(), c"a.b"), Err(ErrorCode::NotFound.raw()));
                REFUSING.store(true, SeqCst);
                let boxed = ConfigBox::new().is_none();
                let shared = ConfigArc::new().is_none();
                let error = Config::new().unwrap_err();
                REFUSING.store(false, SeqCst);
                assert!(boxed && shared);
                assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                assert_eq!(get(kept.as_ref(), c"a.b"), Err(ErrorCode::NotFound.raw()));
            },
        );
    }

    /// Without libgit2's runtime nothing is built, and the error is synthesised without
    /// calling libgit2.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn without_runtime_fails_cleanly() {
        isolated(
            concat!(module_path!(), "::without_runtime_fails_cleanly"),
            || {
                REFUSING.store(true, SeqCst);
                install_refusing();
                assert!(ConfigBox::new().is_none());
                let error = Config::new().unwrap_err();
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                assert_eq!(error.class(), Some(ErrorClass::Invalid));
                assert_eq!(error.message(), c"library has not been initialized");
                // Every parser refuses even otherwise-valid inputs without calling
                // C when the runtime cannot be pinned.
                assert_eq!(parse_bool(None).unwrap_err(), error);
                assert_eq!(parse_i32(Some(c"1")).unwrap_err(), error);
                assert_eq!(parse_i64(Some(c"1")).unwrap_err(), error);
                // So does every constructor and search routine that needs no config.
                let failures = [
                    Config::open(c"unused").map(drop),
                    Config::open_default().map(drop),
                    find_global().map(drop),
                    find_system().map(drop),
                    find_xdg().map(drop),
                    parse_path(c"~/unused").map(drop),
                    lookup_map_value(&[], None).map(drop),
                ];
                for failure in failures {
                    let failure = failure.unwrap_err();
                    assert_eq!(failure.code(), Some(ErrorCode::Generic));
                    assert_eq!(*failure.error(), error);
                }
            },
        );
    }

    /// Out of memory, the view, opening and file-adding routines report libgit2's error
    /// and free what they built (`LSan`), whether the new config or its backend entry
    /// failed to allocate, and the parent stays usable.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn opening_routines_report_allocation_failure() {
        isolated(
            concat!(
                module_path!(),
                "::opening_routines_report_allocation_failure"
            ),
            || {
                install_refusing();
                let dir =
                    std::env::temp_dir().join(format!("crustify-open-oom-{}", std::process::id()));
                drop(std::fs::remove_dir_all(&dir));
                std::fs::create_dir_all(&dir).unwrap();
                let file = |name: &str| {
                    let path = dir.join(name);
                    std::fs::write(&path, "[a]\n b = 1\n").unwrap();
                    CString::new(path.to_str().unwrap()).unwrap()
                };
                let (local, xdg) = (file("local"), file("xdg"));
                let mut parent = Config::open(&local).unwrap();
                parent
                    .as_mut()
                    .add_file(&xdg, ConfigLevel::XDG, None, false)
                    .unwrap();
                // A first error allocates the thread's error state.
                assert_eq!(get(parent.as_ref(), c"a.c"), Err(ErrorCode::NotFound.raw()));
                REFUSING.store(true, SeqCst);
                let failures = [
                    parent.as_ref().open_level(ConfigLevel::LOCAL).map(drop),
                    parent.as_ref().open_global().map(drop),
                    parent.as_mut().open_level(ConfigLevel::LOCAL).map(drop),
                    parent.as_mut().open_global().map(drop),
                    Config::open(&local).map(drop),
                    Config::open_default().map(drop),
                    parent
                        .as_mut()
                        .add_file(&local, ConfigLevel::APP, None, false),
                ];
                REFUSING.store(false, SeqCst);
                // The new config and its two vectors allocate; the backend entry fails.
                ALLOWED_ALLOCATIONS.store(3, SeqCst);
                let entry = parent.as_ref().open_level(ConfigLevel::LOCAL).map(drop);
                ALLOWED_ALLOCATIONS.store(usize::MAX, SeqCst);
                for failure in failures.into_iter().chain([entry]) {
                    let failure = failure.unwrap_err();
                    assert_eq!(failure.class(), Some(ErrorClass::NoMemory), "{failure:?}");
                }
                assert_eq!(get(parent.as_ref(), c"a.b"), Ok(1));
                let view = parent.as_ref().open_global().unwrap();
                assert_eq!(get(view.as_ref(), c"a.b"), Ok(1));
                drop(view);
                drop(parent);
                std::fs::remove_dir_all(dir).unwrap();
            },
        );
    }

    /// A write order of `INT_MAX` levels is refused before C through the public method
    /// too. The levels are calloc'd and never touched, so this costs no memory.
    #[test]
    #[cfg_attr(miri, ignore = "allocates 8 GiB of address space; calls libgit2")]
    fn write_order_refuses_oversized_slices() {
        let raw = vec![0; c_int::MAX as usize];
        let (levels, len) = (raw.as_ptr().cast::<ConfigLevel>(), raw.len());
        // SAFETY: `ConfigLevel` is `#[repr(transparent)]` over `git_config_level_t`;
        // the slice covers the vector's zeroed elements and is only read.
        let levels = unsafe { core::slice::from_raw_parts(levels, len) };
        let mut cfg = Config::new().unwrap();
        let error = cfg.as_mut().set_write_order(levels).unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Invalid));
        assert_eq!(
            error.message(),
            c"write order length must be less than INT_MAX"
        );
    }

    /// The message quotes the value's bytes as they are, as C's `%s` does.
    #[test]
    fn integer_parse_error_quotes_the_value() {
        assert_eq!(
            integer_parse_error(None).as_c_str(),
            c"failed to parse '(null)' as an integer"
        );
        assert_eq!(
            integer_parse_error(Some(c"")).as_c_str(),
            c"failed to parse '' as an integer"
        );
        assert_eq!(
            integer_parse_error(Some(c"1k\xff%s")).as_c_str(),
            c"failed to parse '1k\xff%s' as an integer"
        );
    }

    /// libgit2 sets no error for text after a size suffix: the wrapper reports that
    /// failure in libgit2's own words, not with the thread's earlier error (or "no
    /// error"), and a success leaves no earlier error behind.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn parse_i64_suffix_tail_reports_its_own_error() {
        for value in [c"1kb", c"1kz", c"-2Mx", c"1g ", c"0x10kk", c"1k\xff"] {
            for seeded in [false, true] {
                clear();
                if seeded {
                    set(ErrorClass::Odb, c"an earlier, unrelated failure");
                }
                let error = parse_i64(Some(value)).unwrap_err();
                let expected = [
                    b"failed to parse '".as_slice(),
                    value.to_bytes(),
                    b"' as an integer",
                ];
                assert_eq!(error.class(), Some(ErrorClass::Config), "{value:?}");
                assert_eq!(error.message().to_bytes(), expected.concat(), "{value:?}");
                assert_eq!(last(), error, "the thread's error describes it too");
            }
        }
        // libgit2's own errors pass through unchanged.
        set(ErrorClass::Odb, c"an earlier, unrelated failure");
        let error = parse_i64(Some(c"1x")).unwrap_err();
        assert_eq!(error.message(), c"failed to parse '1x' as an integer");
        let error = parse_i64(None).unwrap_err();
        assert_eq!(error.message(), c"failed to parse '(null)' as an integer");
        set(ErrorClass::Odb, c"an earlier, unrelated failure");
        assert_eq!(parse_i64(Some(c"1k")), Ok(1024));
        assert_eq!(last().class(), Some(ErrorClass::None));
        // The 32-bit and boolean parsers set their own errors over it.
        let error = parse_i32(Some(c"1kb")).unwrap_err();
        assert_eq!(
            error.message(),
            c"failed to parse '1kb' as a 32-bit integer"
        );
        let error = parse_bool(Some(c"1kb")).unwrap_err();
        assert_eq!(error.message(), c"failed to parse '1kb' as a boolean");
    }
    #[test]
    fn layout_and_custom_values() {
        assert_eq!(
            size_of::<ConfigLevel>(),
            size_of::<ffi::git_config_level_t>()
        );
        assert_eq!(
            align_of::<ConfigLevel>(),
            align_of::<ffi::git_config_level_t>()
        );
        for raw in [i32::MIN, -1, 0, 1, 2, 7, 42, i32::MAX] {
            let level = ConfigLevel::from(raw);
            assert_eq!(ffi::git_config_level_t::from(level), raw);
            assert_eq!(format!("{level:?}"), format!("ConfigLevel({raw})"));
        }
        assert!(ConfigLevel::SYSTEM < ConfigLevel::XDG);
        assert!(ConfigLevel::APP < ConfigLevel::from_raw(42));
    }

    #[test]
    fn checked_mapping_modes() {
        for mode in MapType::ALL {
            assert_eq!(MapType::from_raw(mode.raw()), Some(mode));
            assert_eq!(ffi::git_configmap_t::from(mode), mode.raw());
        }
        assert_eq!(MapType::from_raw(4), None);
        assert_eq!(MapType::from_raw(ffi::git_configmap_t::MAX), None);
        assert_ne!(MapType::False, MapType::True);
        assert_eq!(format!("{:?}", MapType::Int32), "Int32");
    }

    #[test]
    #[cfg_attr(miri, ignore = "reads libgit2's last error")]
    fn config_errors_keep_codes() {
        let _config = Config::new().unwrap();
        set(ErrorClass::Config, c"held");
        let locked = ConfigError::from_libgit2(ErrorCode::Locked.raw());
        assert_eq!(locked.code(), Some(ErrorCode::Locked));
        assert_eq!(locked.raw_code(), ffi::GIT_ELOCKED);
        assert_eq!(locked.class(), Some(ErrorClass::Config));
        assert_eq!(locked.raw_class(), ffi::GIT_ERROR_CONFIG.cast_signed());
        assert_eq!(locked.message(), c"held");
        assert_eq!(locked.error().message(), c"held");
        assert_eq!(
            format!("{locked:?}"),
            r#"ConfigError { code: Locked, error: ErrorBuf { class: Config, message: "held" } }"#
        );
        assert_eq!(locked.to_string(), "held");
        assert_eq!(ErrorBuf::from(locked.clone()), *locked.error());
        // A positive or zero code is still a failure; an unnamed one is kept raw.
        assert_eq!(
            ConfigError::from_libgit2(0).code(),
            Some(ErrorCode::Generic)
        );
        assert_eq!(ConfigError::from_libgit2(7).raw_code(), ffi::GIT_ERROR);
        let unnamed = ConfigError::from_libgit2(-1000);
        assert_eq!((unnamed.code(), unnamed.raw_code()), (None, -1000));
        assert!(format!("{unnamed:?}").starts_with("ConfigError { code: -1000,"));
    }

    /// A store's own failure: `GIT_ERROR` until given another code, never success, and
    /// raised verbatim (an OS-class message gains no `errno` suffix) with its code.
    #[test]
    #[cfg_attr(miri, ignore = "raises libgit2 errors")]
    fn store_errors_take_codes_and_raise_verbatim() {
        let _config = Config::new().unwrap();
        let error = ConfigError::new(ErrorClass::Os, c"store refused %s");
        assert_eq!(error.code(), Some(ErrorCode::Generic));
        assert_eq!(
            (error.class(), error.message()),
            (Some(ErrorClass::Os), c"store refused %s")
        );
        let missing = error.clone().with_code(ErrorCode::NotFound);
        assert_eq!(missing.raw_code(), ffi::GIT_ENOTFOUND);
        // `Ok` would read as success: the error keeps whatever code it had.
        assert_eq!(
            missing.clone().with_code(ErrorCode::Ok).code(),
            Some(ErrorCode::NotFound)
        );
        assert_eq!(
            error.clone().with_code(ErrorCode::Ok).code(),
            Some(ErrorCode::Generic)
        );
        clear();
        assert_eq!(missing.raise(), ffi::GIT_ENOTFOUND);
        let raised = last();
        assert_eq!(raised.raw_class(), ffi::GIT_ERROR_OS.cast_signed());
        assert_eq!(raised.message(), c"store refused %s");
    }

    /// libgit2 sets no error when a delete finds no writer: the wrappers report that
    /// failure themselves, not with the thread's earlier error (or "no error"), for an
    /// empty config and a snapshot alike, and through a borrowed exclusive handle.
    /// The backend's own errors pass through, and a success leaves no earlier error.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn readonly_deletes_report_their_own_error() {
        let mut empty = Config::new().unwrap();
        let mut snapshot = empty.as_ref().snapshot().unwrap();
        for config in [&mut empty, &mut snapshot] {
            for seeded in [false, true] {
                for multivar in [false, true] {
                    clear();
                    if seeded {
                        set(ErrorClass::Odb, c"an earlier, unrelated failure");
                    }
                    let mut m = config.as_mut();
                    let error = if multivar {
                        m.delete_multivar(c"a.b%s", c".*").unwrap_err()
                    } else {
                        m.delete_entry(c"a.b%s").unwrap_err()
                    };
                    assert_eq!(error.code(), Some(ErrorCode::ReadOnly));
                    assert_eq!(error.class(), Some(ErrorClass::Config));
                    assert_eq!(
                        error.message(),
                        c"cannot delete 'a.b%s': the configuration is read-only"
                    );
                    assert_eq!(
                        last(),
                        *error.error(),
                        "the thread's error describes it too"
                    );
                }
            }
        }
        let file =
            std::env::temp_dir().join(format!("crustify-delete-readonly-{}", std::process::id()));
        std::fs::write(&file, b"[a]\n b = 1\n m = 1\n m = 2\n").unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        let mut writable = Config::open(&path).unwrap();
        set(ErrorClass::Odb, c"an earlier, unrelated failure");
        let error = writable.as_mut().delete_entry(c"a.missing").unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::NotFound));
        assert_eq!(error.message(), c"could not find key 'a.missing' to delete");
        let error = writable.as_mut().delete_entry(c"a.m").unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Config));
        set(ErrorClass::Odb, c"an earlier, unrelated failure");
        writable.as_mut().delete_multivar(c"a.m", c"2").unwrap();
        assert_eq!(last().class(), Some(ErrorClass::None));
        set(ErrorClass::Odb, c"an earlier, unrelated failure");
        writable.as_mut().delete_entry(c"a.b").unwrap();
        assert_eq!(last().class(), Some(ErrorClass::None));
        drop(writable);
        std::fs::remove_file(file).unwrap();
    }

    /// The message quotes the name's bytes as they are; a `%` stays literal.
    #[test]
    fn readonly_delete_error_quotes_the_name() {
        assert_eq!(
            readonly_delete_error(c"").as_c_str(),
            c"cannot delete '': the configuration is read-only"
        );
        assert_eq!(
            readonly_delete_error(c"a.\xff%s").as_c_str(),
            c"cannot delete 'a.\xff%s': the configuration is read-only"
        );
    }

    /// The iterator constructors and `advance_entry`, homed with them here.
    mod iterators {
        use std::cell::Cell;
        use std::rc::Rc;

        use super::*;

        /// Constructors fail cleanly when libgit2's allocator refuses: the all
        /// iterator's (and glob's) only allocation, and the multivar name copy
        /// after C has built both iterators. C's leak when the multivar shell
        /// itself is refused (`config.c:1139`, its second allocation) is skipped.
        #[test]
        #[cfg_attr(miri, ignore = "installs a libgit2 allocator")]
        #[expect(
            clippy::items_after_statements,
            reason = "each helper item sits beside the steps that use it"
        )]
        fn constructors_report_allocation_failure() {
            isolated(
                concat!(module_path!(), "::constructors_report_allocation_failure"),
                || {
                    install_refusing();
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
                        ALLOWED_ALLOCATIONS.store(granted, SeqCst);
                        let result = build(config.as_ref());
                        ALLOWED_ALLOCATIONS.store(usize::MAX, SeqCst);
                        let error = result.unwrap_err();
                        assert_eq!(error.code(), Some(ErrorCode::Generic));
                        assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                        let mut iter = build(config.as_ref()).unwrap();
                        assert!(iter.as_mut().advance_entry().unwrap().is_none());
                    }
                },
            );
        }

        /// A Rust-owned iterator allocation whose `next` replays `script`: 0
        /// lends `entry`, anything else is returned as the status, without
        /// writing the output. A failure sets its own thread error first.
        #[repr(C)]
        struct Scripted {
            base: ffi::git_config_iterator,
            entry: ffi::git_config_backend_entry,
            script: &'static [c_int],
            calls: usize,
            releases: Rc<Cell<usize>>,
        }

        impl Scripted {
            unsafe extern "C" fn next(
                out: *mut *mut ffi::git_config_backend_entry,
                p: *mut ffi::git_config_iterator,
            ) -> c_int {
                let p = p.cast::<Self>();
                {
                    // SAFETY: this callback belongs only to Scripted allocations, with
                    // the base at offset zero; C gives exclusive access for the call
                    // and out is a writable slot. Only by-value fields are read.
                    let calls = unsafe { addr_of_mut!((*p).calls) };
                    // SAFETY: as above.
                    let n = unsafe { calls.read() };
                    // SAFETY: as above.
                    unsafe { calls.write(n + 1) };
                    // SAFETY: as above.
                    let script_ptr = unsafe { addr_of!((*p).script) };
                    // SAFETY: as above.
                    let status = unsafe {
                        script_ptr
                            .read()
                            .get(n)
                            .copied()
                            .unwrap_or(ffi::GIT_ITEROVER)
                    };
                    match status {
                        0 => {
                            // SAFETY: as above; the projection forms no reference to the fixture.
                            let entry = unsafe { &raw mut (*p).entry };
                            // SAFETY: as above, `out` is a writable slot.
                            unsafe { out.write(entry) };
                        }
                        ffi::GIT_ITEROVER => {}
                        _ => set(ErrorClass::Config, c"scripted failure"),
                    }
                    status
                }
            }

            unsafe extern "C" fn free(p: *mut ffi::git_config_iterator) {
                // SAFETY: adoption transfers exactly this Box, base at offset zero;
                // dispatch runs once, after every lent entry's borrow has ended.
                let value = unsafe { Box::from_raw(p.cast::<Self>()) };
                value.releases.set(value.releases.get() + 1);
            }

            fn adopt(script: &'static [c_int], releases: Rc<Cell<usize>>) -> IteratorBox<'static> {
                let value = Box::new(Self {
                    base: ffi::git_config_iterator {
                        backend: ptr::null_mut(),
                        flags: 0,
                        next: Some(Self::next),
                        free: Some(Self::free),
                    },
                    entry: ffi::git_config_backend_entry {
                        entry: ffi::git_config_entry {
                            name: c"scripted.key".as_ptr(),
                            value: ptr::null(),
                            backend_type: c"scripted".as_ptr(),
                            origin_path: ptr::null(),
                            include_depth: 0,
                            level: ffi::GIT_CONFIG_LEVEL_APP,
                        },
                        free: None, // Lent only, never acquired or released.
                    },
                    script,
                    calls: 0,
                    releases,
                });
                // SAFETY: one initialized, uniquely owned enclosing Box whose free
                // callback matches; static strings, and no config dependency.
                unsafe { IteratorBox::from_c(Box::into_raw(value).cast()) }.unwrap()
            }
        }

        /// Rust-only adapter/provenance test over the setup `foreach_match` uses:
        /// the walk below calls the slots in C's order, through the lent base.
        /// Manually dispatches the allocation's Rust releaser instead of libgit2's
        /// dispatcher so Miri can execute it.
        #[test]
        fn backend_walk_adapter_borrowed_bases_and_lending() {
            let releases = Rc::new(Cell::new(0));
            let inner = Scripted::adopt(&[0, ffi::GIT_ITEROVER], releases.clone());
            let (rc, state) = PreparedBackendWalk::run(inner, true, |backend| {
                // SAFETY: the lent base is initialized; slots are read by value.
                let iterator_ptr = unsafe { addr_of!((*backend).iterator) };
                // SAFETY: as above.
                let make_iterator = unsafe { iterator_ptr.read() }.unwrap();
                let mut iterator = ptr::null_mut();
                // SAFETY: the adapter's own callback on its base; writable out.
                assert_eq!(unsafe { make_iterator(&raw mut iterator, backend) }, 0);
                assert_eq!(
                    iterator.cast::<u8>(),
                    backend
                        .cast::<u8>()
                        .wrapping_add(core::mem::offset_of!(PreparedBackendWalk<'_>, iterator))
                );
                // SAFETY: the iterator base handed out above is initialized.
                let next_ptr = unsafe { addr_of!((*iterator).next) };
                // SAFETY: as above.
                let read2 = unsafe { next_ptr.read() };
                // SAFETY: as above.
                let free_ptr = unsafe { addr_of!((*iterator).free) };
                // SAFETY: as above.
                let (next, free) = unsafe { (read2, free_ptr.read()) };
                let (next, free) = (next.unwrap(), free.unwrap());
                let mut entry = ptr::null_mut();
                // SAFETY: the adapter's matched iterator; writable slot. The inner
                // Rust callback lends a static-string entry without foreign calls.
                assert_eq!(unsafe { next(&raw mut entry, iterator) }, 0);
                // SAFETY: the successful advance lends an initialized live entry
                // until the next advance. A shared view does not release its storage.
                let view = unsafe { EntryRef::from_ptr(entry.cast()) }.unwrap();
                assert_eq!(view.name(), Some(c"scripted.key"));
                // SAFETY: the entry borrow is over; same iterator and slot.
                assert_eq!(unsafe { next(&raw mut entry, iterator) }, ffi::GIT_ITEROVER);
                // SAFETY: this base is borrowed stack storage; the matching free is a
                // no-op and neither consumes nor changes the Rust owner.
                unsafe { free(iterator) };
                assert_eq!(releases.get(), 0);
                7
            });
            assert_eq!(rc, 7);
            assert!(state.failure.is_none());
            assert_eq!(releases.get(), 0);
            let raw = state.inner.into_c();
            // SAFETY: consumes exactly the complete Scripted Box created above,
            // via its own Rust callback, after all borrowed entry views ended.
            unsafe { Scripted::free(raw) };
            assert_eq!(releases.get(), 1);
        }

        #[repr(C)]
        struct WalkBackend {
            base: ffi::git_config_backend,
            script: &'static [c_int],
            releases: Rc<Cell<usize>>,
        }
        impl WalkBackend {
            unsafe extern "C" fn iterator(
                out: *mut *mut ffi::git_config_iterator,
                backend: *mut ffi::git_config_backend,
            ) -> c_int {
                // SAFETY: callback only belongs to this Rust-owned offset-zero
                // fixture; state is immutable for this synchronous exclusive call.
                let state = unsafe { &*backend.cast::<Self>() };
                if state.script == [-100] {
                    set(ErrorClass::Config, c"mock iterator construction failure");
                    return -7;
                }
                let script = if state.script == [42] {
                    &[0, ffi::GIT_ITEROVER]
                } else {
                    state.script
                };
                let mut owner = Scripted::adopt(script, state.releases.clone());
                if state.script == [42] {
                    // SAFETY: the owner is the uniquely owned, complete Scripted fixture, before
                    // any entry is lent; the projection forms no reference to it.
                    let name = unsafe {
                        &raw mut (*owner.as_mut().as_mut_ptr().cast::<Scripted>())
                            .entry
                            .entry
                            .name
                    };
                    // SAFETY: NULL is a valid optional name for EntryRef.
                    unsafe { name.write(ptr::null()) };
                }
                // SAFETY: C supplies writable out; transfers one complete iterator
                // allocation with matching callbacks and only static dependencies.
                unsafe { out.write(owner.into_c()) };
                0
            }

            fn new(script: &'static [c_int], releases: Rc<Cell<usize>>) -> Self {
                Self {
                    base: ffi::git_config_backend {
                        version: 1,
                        iterator: Some(Self::iterator),
                        ..Default::default()
                    },
                    script,
                    releases,
                }
            }

            fn as_mut(&mut self) -> BackendMut<'_> {
                // SAFETY: exclusive borrowed initialized backend fixture; this
                // allocation-compatible iterator needs no separate open, and no
                // owner adopts the base or dispatches its absent free callback.
                unsafe { BackendMut::from_ptr(addr_of_mut!(self.base)) }.unwrap()
            }
        }
        #[test]
        #[cfg_attr(miri, ignore = "calls C walk and iterator release")]
        fn backend_walk_reports_masked_next_errors_and_cleans_failure_paths() {
            let _runtime = Config::new().unwrap();
            for script in [&[-100][..], &[-7][..], &[0, -7][..], &[3][..]] {
                for pattern in [None, Some(c".*")] {
                    let releases = Rc::new(Cell::new(0));
                    let mut backend = WalkBackend::new(script, releases.clone());
                    let mut calls = 0;
                    let mut f = |_: EntryRef<'_>| {
                        calls += 1;
                        ControlFlow::Continue(())
                    };
                    let mut callback = ForeachCallback::new(&mut f);
                    let error = backend
                        .as_mut()
                        .foreach_match(pattern, &mut callback)
                        .unwrap_err();
                    assert_eq!(
                        error.raw_code(),
                        if script == [3] { ffi::GIT_ERROR } else { -7 }
                    );
                    assert_eq!(error.class(), Some(ErrorClass::Config));
                    assert_eq!(callback.stopped(), None);
                    drop(callback);
                    assert_eq!(calls, usize::from(script[0] == 0));
                    assert_eq!(releases.get(), usize::from(script != [-100]));
                }
            }
            let releases = Rc::new(Cell::new(0));
            let mut backend = WalkBackend::new(&[0, ffi::GIT_ITEROVER], releases.clone());
            let mut f = |_: EntryRef<'_>| ControlFlow::Continue(());
            let mut callback = ForeachCallback::new(&mut f);
            assert!(
                backend
                    .as_mut()
                    .foreach_match(Some(c"["), &mut callback)
                    .is_err()
            );
            assert_eq!(releases.get(), 1); // even regex compilation failure frees inner
            assert_eq!(
                backend.as_mut().foreach_match(None, &mut callback).unwrap(),
                ControlFlow::Continue(())
            );
            assert_eq!(releases.get(), 2);
            let mut nameless = WalkBackend::new(&[42], releases.clone());
            let mut f = |entry: EntryRef<'_>| {
                assert_eq!(entry.name(), None);
                ControlFlow::Continue(())
            };
            assert_eq!(
                nameless
                    .as_mut()
                    .foreach_match(None, &mut ForeachCallback::new(&mut f))
                    .unwrap(),
                ControlFlow::Continue(())
            );
            let error = nameless
                .as_mut()
                .foreach_match(Some(c".*"), &mut ForeachCallback::new(&mut f))
                .unwrap_err();
            assert_eq!(error.raw_code(), ffi::GIT_EINVALIDSPEC);
            assert_eq!(
                error.message(),
                c"config backend iterator returned an entry without a name"
            );
            let mut inline = crate::libgit2::config::sys::config::Backend::new();
            assert_eq!(
                inline
                    .as_mut()
                    .foreach_match(None, &mut callback)
                    .unwrap_err()
                    .message(),
                c"config backend does not implement 'iterator'"
            );
        }

        /// `git_config_next` dispatches the allocation's own callback and lends
        /// the public entry without acquiring it. Failures keep their code and
        /// thread error, a positive status is reported as `GIT_ERROR`, and the
        /// iterator stays usable after either and after the end. The allocation
        /// is released once, by its own callback.
        #[test]
        #[cfg_attr(miri, ignore = "calls git_config_next and iterator_free")]
        fn advance_entry_dispatches_statuses_and_lends() {
            drop(Config::new().unwrap()); // Pins the runtime for error reporting.
            let releases = Rc::new(Cell::new(0));
            let script = &[0, -7, 3, ffi::GIT_ITEROVER, 0];
            let mut owner = Scripted::adopt(script, releases.clone());
            {
                let mut m = owner.as_mut();
                let entry = m.advance_entry().unwrap().unwrap();
                assert_eq!(entry.name(), Some(c"scripted.key"));
                assert_eq!(entry.value(), None);
                assert_eq!(entry.backend_type(), Some(c"scripted"));
                assert_eq!(entry.level(), ConfigLevel::APP);
                let error = m.advance_entry().unwrap_err();
                assert_eq!(error.raw_code(), -7);
                assert_eq!(error.class(), Some(ErrorClass::Config));
                assert_eq!(error.message(), c"scripted failure");
                let error = m.advance_entry().unwrap_err();
                assert_eq!(error.code(), Some(ErrorCode::Generic));
                assert_eq!(error.message(), c"scripted failure");
                assert!(m.advance_entry().unwrap().is_none());
                assert!(m.advance_entry().unwrap().is_some());
                assert!(m.advance_entry().unwrap().is_none());
                assert!(m.advance_entry().unwrap().is_none());
            }
            assert_eq!(releases.get(), 0);
            drop(owner);
            assert_eq!(releases.get(), 1);
        }
    }
}
