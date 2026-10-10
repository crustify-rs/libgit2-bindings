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
//! - A [`ConfigBox`] is the only reference: what `git_config_new` (through
//!   [`Config::new`]) and the opening routines (`git_config_open_ondisk`,
//!   `git_config_open_default`, `git_config_open_level`, `git_config_open_global`)
//!   and snapshots (`git_config_snapshot`, `git_repository_config_snapshot`) hand out.
//!   It alone reaches the exclusive [`ConfigMut`].
//! - A [`ConfigArc`] is one of several references: what `git_repository_config`
//!   hands out, the repository holding another (and writing through it, as
//!   `git_repository_set_workdir` does), or a box once shared with
//!   [`From`](ConfigArc::from). It reaches only the shared [`ConfigRef`]. libgit2
//!   publishes no up-ref, so it does not clone.
//!
//! A sole reference stops being one when C takes another: a wrapper of
//! `git_repository_set_config`, which up-refs the config into the repository, must
//! take a [`ConfigArc`], not a [`ConfigBox`]'s handle.
//!
//! A repository's config outlives the repository: `git_repository_free` disowns it
//! (`GIT_REFCOUNT_OWN(config, NULL)`) before releasing its own reference, so a
//! [`ConfigArc`] may be used and dropped after it.
//!
//! The struct is opaque (its body is in `src/libgit2/config.h`), so there are no field
//! accessors: the handles carry the routines of `git2/config.h`, which their own runs
//! add. `Config` asserts neither `Send` nor `Sync`: libgit2 documents only snapshots
//! as shareable across threads, nothing synchronises the backend lists a config
//! mutates, and a repository-owned config reads its owner (`GIT_REFCOUNT_OWNER`) in
//! `git_config_set_string` to clear the owner's cache, racing a release of that
//! repository on another thread.

use core::ptr::{self, NonNull};

use ffibox::{CArc, CBox, CNew, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

define_ctype!(
    /// A configuration (`git_config`): an opaque, reference-counted object of
    /// libgit2's, reached only behind a pointer.
    ///
    /// Owned as a [`ConfigBox`] when the reference is the only one, which
    /// [`new`](Self::new) builds, or as a [`ConfigArc`] when others exist; see the
    /// [module docs](self). Neither crosses threads.
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
    /// Wraps: git_config
    Config, ConfigRef, ConfigMut, ffi::git_config
);

/// Releases one reference to a config through `git_config_free`, which frees the
/// config, its backend list and every backend it alone holds once the count reaches
/// zero and no repository owns it; builds one through `git_config_new`.
///
/// Wraps: git_config_free
#[derive(Clone, Copy, Debug, Default)]
pub struct ConfigFree;

impl_cdrop!(ConfigFree, Config, ffi::git_config_free);

// SAFETY: `git_config_new` hands out a fresh `git__calloc`'d config with empty backend
// lists and a reference count of 1, owned by no repository: one reference, which
// `git_config_free` releases, freeing it. It writes `out` only on success, so a failure
// (out of memory, or libgit2 not initialisable) is `None`.
unsafe impl CNew<Config> for ConfigFree {
    fn c_new(&self) -> Option<NonNull<Config>> {
        if !runtime_pinned() {
            return None;
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot. The runtime is pinned, so the allocation
        // goes through an initialised allocator and a failure is reported on live
        // thread-local state.
        let rc = unsafe { ffi::git_config_new(&mut out) };
        if rc < 0 {
            return None;
        }
        NonNull::new(out.cast())
    }
}

/// The only reference to a [`Config`], released by `git_config_free` on drop.
pub type ConfigBox = CBox<Config, ConfigFree>;

/// One of several references to a [`Config`], released by `git_config_free` on drop;
/// not `Clone`, since libgit2 publishes no up-ref.
pub type ConfigArc = CArc<Config, ConfigFree>;

impl Config {
    /// An empty configuration, with no backends: every lookup misses, and every
    /// write fails until a backend is added.
    ///
    /// Pins the runtime first. Fails, with libgit2's error, when the config cannot be
    /// allocated or libgit2 cannot be initialised.
    ///
    /// Wraps: git_config_new
    pub fn new() -> Result<ConfigBox, ErrorBuf> {
        ConfigBox::new().ok_or_else(last)
    }
}

// A `Config` is only ever behind a pointer, so the owners are pointer-sized, and an
// optional one is a nullable `git_config *`.
const _: () = assert!(
    size_of::<ConfigBox>() == size_of::<*mut ffi::git_config>()
        && size_of::<ConfigArc>() == size_of::<*mut ffi::git_config>()
        && size_of::<Option<ConfigBox>>() == size_of::<*mut ffi::git_config>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::errors::{ErrorClass, ErrorCode};
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use core::ffi::{CStr, c_int, c_void};
    use core::mem::size_of;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

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

    fn install_refusing() {
        let allocator = Allocator::new::<Refusing>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// `git_config_get_int32` on `config`: the value, or the return code.
    fn get(config: ConfigRef<'_>, name: &CStr) -> Result<i32, c_int> {
        let mut value = 0;
        // SAFETY: `config` is live for the call, which only reads it; `value` is a
        // writable slot and `name` NUL-terminated.
        let rc = unsafe { ffi::git_config_get_int32(&mut value, config.as_ptr(), name.as_ptr()) };
        if rc < 0 { Err(rc) } else { Ok(value) }
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ConfigBox>(), size_of::<*mut ffi::git_config>());
        assert_eq!(size_of::<ConfigArc>(), size_of::<*mut ffi::git_config>());
        assert_eq!(
            size_of::<Option<ConfigArc>>(),
            size_of::<*mut ffi::git_config>()
        );
        assert_eq!(
            size_of::<ConfigRef<'_>>(),
            size_of::<*const ffi::git_config>()
        );
        assert_eq!(
            size_of::<ConfigMut<'_>>(),
            size_of::<*const ffi::git_config>()
        );
        assert_eq!(size_of::<ConfigFree>(), 0);
    }

    /// Every construction path builds an empty config, which misses every lookup and
    /// refuses writes, and is freed once (LSan flags a leak, ASan a double free).
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
        assert_eq!(format!("{:p}", other), format!("{:p}", other.as_c_ptr()));
    }

    /// A repository's config, adopted as a shared reference, stays readable after the
    /// repository is freed, and the last release frees it (ASan flags a use after free
    /// or a double free, LSan a leak).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn repository_config_outlives_its_repository() {
        assert!(runtime_pinned());
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/resources/testrepo.git\0"
        );
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
        let adopt = || {
            let mut out = ptr::null_mut();
            // SAFETY: `out` is a writable slot, `repo` open.
            assert_eq!(unsafe { ffi::git_repository_config(&mut out, repo) }, 0);
            // SAFETY: `git_repository_config` hands out one up-ref'd reference to the
            // repository's config, released by `git_config_free`; the repository holds
            // another, so it is shared.
            unsafe { ConfigArc::from_c(out) }.unwrap()
        };
        let (first, second) = (adopt(), adopt());
        assert!(ConfigArc::ptr_eq(&first, &second));
        assert_eq!(get(first.as_ref(), c"core.repositoryformatversion"), Ok(0));
        drop(first);
        // SAFETY: `repo` is open and freed once; the config outlives it.
        unsafe { ffi::git_repository_free(repo) };
        assert_eq!(get(second.as_ref(), c"core.repositoryformatversion"), Ok(0));
        assert_eq!(
            get(second.as_ref(), c"core.missing"),
            Err(ErrorCode::NotFound.raw())
        );
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
            },
        );
    }
}
