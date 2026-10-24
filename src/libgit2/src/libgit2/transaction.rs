//! C-owned transactions. Reference transactions borrow their repository; configuration
//! transactions exclusively borrow their config and retain its backend lock until drop.
//!
//! C owns both allocation variants: reference transactions live inside a pool, while
//! config transactions are individually allocated. `git_transaction_free` selects the
//! matching release using C's private discriminator. No clone exists, no fields are
//! published, and neither form is `Send` or `Sync`.

use core::fmt;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop};

use crate::ffi;
use crate::libgit2::src::libgit2::config::{ConfigMut, ConfigRef};
use crate::libgit2::src::libgit2::repository::RepositoryRef;
use crate::libgit2::src::util::errors::{ErrorBuf, last, runtime_pinned};

/// Opaque reference transaction, borrowing its repository for `'r`.
///
/// The refdb reference C retains does not keep that repository alive. Owners and
/// handles therefore cannot escape the repository's borrow:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::transaction::{Transaction, TransactionBox};
/// use libgit2::libgit2::src::libgit2::repository::Repository;
/// fn escape() -> TransactionBox<'static> {
///     let repo = Repository::open(c".").unwrap();
///     Transaction::new(repo.as_ref()).unwrap()
/// }
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::transaction::TransactionMut;
/// fn shrink<'a, 'r>(m: TransactionMut<'a, 'static>) -> TransactionMut<'a, 'r> { m }
/// ```
///
/// Neither an owner nor a shared owner crosses threads:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::transaction::TransactionBox;
/// fn require_send<T: Send>() {}
/// require_send::<TransactionBox<'static>>();
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::transaction::TransactionBox;
/// fn require_sync<T: Sync>() {}
/// require_sync::<TransactionBox<'static>>();
/// ```
///
/// Wraps: `git_transaction`
/// Handwritten
#[repr(transparent)]
pub struct Transaction<'r>(ffi::git_transaction, PhantomData<(*const (), &'r ())>);

/// Shared, copyable borrow of a reference transaction.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct TransactionRef<'a, 'r>(CBorrowedPtr<'a, Transaction<'r>>);

/// Exclusive borrow of a reference transaction; reborrows freeze this handle.
///
/// Handwritten
#[repr(transparent)]
pub struct TransactionMut<'a, 'r>(TransactionRef<'a, 'r>, PhantomData<&'a mut Transaction<'r>>);

// SAFETY: the layout and both handles are transparent over the C type and
// CBorrowedPtr respectively, with only zero-sized markers and no handle Drop.
// No Rust reference covers C memory. The mutable handle is invariant in Transaction.
unsafe impl<'r> CCell for Transaction<'r> {
    type C = ffi::git_transaction;
    type Mut<'a>
        = TransactionMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = TransactionRef<'a, 'r>
    where
        Self: 'a;
}

impl TransactionRef<'_, '_> {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_transaction {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> TransactionMut<'_, 'r> {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_transaction {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// A shared handle over the same object, borrowed from this one.
    #[must_use]
    pub fn as_ref(&self) -> TransactionRef<'_, 'r> {
        self.0
    }

    /// An exclusive handle over the same object, reborrowed from this one.
    pub fn as_mut(&mut self) -> TransactionMut<'_, 'r> {
        TransactionMut(self.0, PhantomData)
    }
}

impl fmt::Debug for TransactionRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transaction").finish_non_exhaustive()
    }
}

impl fmt::Debug for TransactionMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases pending locks without committing, then releases C's storage. The
/// discriminator selects pool teardown for references or free for configuration.
///
/// Wraps: `git_transaction_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct TransactionFree;

// SAFETY: C's destructor accepts both initialized transaction variants, releases
// each remaining lock exactly once and frees the matching allocation. It does not
// release the borrowed repository/config, which remain live through the owner.
unsafe impl<'r> CDrop<Transaction<'r>> for TransactionFree {
    unsafe fn c_drop(&self, ptr: NonNull<Transaction<'r>>) {
        // SAFETY: the caller transfers one live transaction allocated by C, with
        // its parent still alive. Transaction has the C type's layout.
        unsafe { ffi::git_transaction_free(ptr.as_ptr().cast()) }
    }
}

/// Sole owner of a reference transaction. Dropping it aborts pending changes.
pub type TransactionBox<'r> = CBox<Transaction<'r>, TransactionFree>;

impl<'r> Transaction<'r> {
    /// Creates a reference transaction without locking any reference. The
    /// repository and its refdb backend must remain alive until the owner drops.
    /// This borrow enforces that keepalive; no raw parent pointer is exposed.
    ///
    /// The first call on a repository opens its reference database. If that runs
    /// out of memory, libgit2 crashes rather than failing: `git_refdb_open` frees
    /// its uninitialised `db` when `git_refdb_new` cannot allocate it.
    ///
    /// Wraps: `git_transaction_new`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's error when the reference database cannot be opened or the transaction
    /// cannot be allocated.
    pub fn new(repo: RepositoryRef<'r>) -> Result<TransactionBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: out is writable and repo is live for 'r; C stores that borrow
        // and obtains its own refdb reference. On failure it cleans its pool and
        // leaves out untouched. The runtime remains initialized through teardown.
        let rc = unsafe { ffi::git_transaction_new(&raw mut out, repo.as_ptr().cast_mut()) };
        if rc < 0 {
            return Err(last());
        }
        // SAFETY: success hands out an initialized reference transaction solely
        // owned here, freed through TransactionFree before repo's borrow ends.
        unsafe { TransactionBox::from_c(out) }.ok_or_else(last)
    }
}

/// Configuration transaction, holding the config's exclusive borrow and backend
/// lock. Unlike a reference transaction it never exposes reference-operation handles.
/// Staged-write wrappers can operate on this owner without exposing a second lock.
/// Dropping this owner aborts changes and releases the backend's retained reference.
///
/// Its borrow cannot outlive the config, and cannot be aliased:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::config::Config;
/// let mut config = Config::new().unwrap();
/// let tx = config.as_mut().lock().unwrap();
/// drop(config);
/// drop(tx);
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::config::Config;
/// let mut config = Config::new().unwrap();
/// let tx = config.as_mut().lock().unwrap();
/// let _alias = config.as_mut();
/// drop(tx);
/// ```
///
/// Handwritten
pub struct ConfigTransaction<'c, 'r> {
    // Fields drop in declaration order: release the backend lock before the
    // config borrow. Never lend this common raw owner as a reference transaction.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) transaction: TransactionBox<'c>,
    pub(crate) config: ConfigMut<'c, 'r>,
}

impl<'r> ConfigTransaction<'_, 'r> {
    /// The locked configuration the transaction writes through.
    #[must_use]
    pub fn config(&self) -> ConfigRef<'_, 'r> {
        self.config.as_ref()
    }

    pub(crate) fn config_mut(&mut self) -> ConfigMut<'_, 'r> {
        self.config.as_mut()
    }

    // The later commit-symbol wrapper must consume this owner: C clears cfg and
    // cfg_data after commit, so committing a config transaction twice is invalid.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_transaction {
        self.transaction.as_c_ptr()
    }
}

impl fmt::Debug for ConfigTransaction<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigTransaction").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn layouts_and_handle_reborrows() {
        assert_eq!(
            size_of::<Transaction<'_>>(),
            size_of::<ffi::git_transaction>()
        );
        assert_eq!(
            align_of::<Transaction<'_>>(),
            align_of::<ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<TransactionBox<'_>>(),
            size_of::<*mut ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<Option<TransactionBox<'_>>>(),
            size_of::<*mut ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<TransactionRef<'_, '_>>(),
            size_of::<*mut ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<TransactionMut<'_, '_>>(),
            size_of::<*mut ffi::git_transaction>()
        );
        // SAFETY: bindgen represents this opaque type as a zero-sized byte array,
        // for which zero is valid. It is never passed to C or read as a C body.
        let mut raw = unsafe { core::mem::zeroed::<ffi::git_transaction>() };
        let p = NonNull::from(&mut raw).cast::<Transaction<'_>>();
        // SAFETY: raw is initialized opaque Rust-owned storage, live throughout
        // this test. No C function is called and no handle dereferences it.
        let shared = TransactionRef(unsafe { CBorrowedPtr::new(p) });
        let mut exclusive = TransactionMut(shared, PhantomData);
        assert_eq!(exclusive.as_mut().as_mut_ptr(), p.as_ptr().cast());
        assert_eq!(exclusive.as_ref().as_ptr(), p.as_ptr().cast());
        assert_eq!(format!("{exclusive:?}"), "Transaction { .. }");
    }
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn config_commit_clears_release_state() {
        use crate::libgit2::src::libgit2::config::Config;
        let path =
            std::env::temp_dir().join(format!("crustify-unit-tx-config-{}", std::process::id()));
        let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        drop(std::fs::remove_file(&path));
        let mut config = Config::new().unwrap();
        // SAFETY: a live sole config, writable file backend and NUL-terminated path;
        // no repository borrow is retained by this backend.
        unsafe {
            assert_eq!(
                ffi::git_config_add_file_ondisk(
                    config.as_c_ptr(),
                    cpath.as_ptr(),
                    ffi::GIT_CONFIG_LEVEL_LOCAL,
                    ptr::null(),
                    0
                ),
                0
            );
        };
        // SAFETY: as above.
        unsafe {
            assert_eq!(
                ffi::git_config_set_int32(config.as_c_ptr(), c"a.b".as_ptr(), 1),
                0
            );
        };
        let mut tx = config.as_mut().lock().unwrap();
        // SAFETY: the exclusive reborrow keeps the config alive and the setter
        // stages a scalar copy. Commit is performed once, before dropping tx.
        unsafe {
            assert_eq!(
                ffi::git_config_set_int32(tx.config_mut().as_mut_ptr(), c"a.b".as_ptr(), 2),
                0
            );
        };
        // SAFETY: as above.
        unsafe { assert_eq!(ffi::git_transaction_commit(tx.as_mut_ptr()), 0) };
        drop(tx); // Must not unlock again after C cleared cfg/cfg_data.
        assert!(std::fs::read_to_string(&path).unwrap().contains('2'));
        drop(config);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adoption_and_pathless_construction() {
        use crate::libgit2::src::libgit2::repository::Repository;
        let repo = Repository::new().unwrap();
        // Construction does not require a repository path and locks nothing.
        let tx = Transaction::new(repo.as_ref()).unwrap();
        let raw = tx.into_c();
        // SAFETY: adopts exactly the sole reference transaction just surrendered;
        // its original repository remains live through drop.
        let adopted = unsafe { TransactionBox::from_c(raw) }.unwrap();
        drop(adopted);
        // SAFETY: NULL is refused without creating an owner or running release.
        assert!(unsafe { TransactionBox::from_c(ptr::null_mut()) }.is_none());
        // SAFETY: the C releaser explicitly accepts NULL as a no-op.
        unsafe { ffi::git_transaction_free(ptr::null_mut()) };
    }

    mod out_of_memory {
        use core::ffi::{CStr, c_void};
        use std::process::Command;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering::SeqCst;

        use super::*;
        use crate::libgit2::src::libgit2::config::Config;
        use crate::libgit2::src::libgit2::repository::Repository;
        use crate::libgit2::src::util::alloc::{Alloc, Allocator};
        use crate::libgit2::src::util::errors::{ErrorClass, ErrorCode};

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, which installs its own allocator.
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
            // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with
            // every member set; libgit2 copies it before anything is allocated.
            let rc = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                    allocator.as_ref().as_ptr(),
                )
            };
            assert_eq!(rc, 0);
        }

        /// Both constructors report an allocation failure as an error, build no owner,
        /// leave no lock behind, and succeed once memory is available again.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn constructors_report_allocation_failure() {
            isolated(
                concat!(module_path!(), "::constructors_report_allocation_failure"),
                || {
                    install_refusing();
                    let repo = Repository::new().unwrap();
                    let path = std::env::temp_dir()
                        .join(format!("crustify-unit-tx-oom-{}", std::process::id()));
                    drop(std::fs::remove_file(&path));
                    let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
                    let mut config = Config::new().unwrap();
                    // SAFETY: a live sole config and a NUL-terminated path; no repository.
                    let rc = unsafe {
                        ffi::git_config_add_file_ondisk(
                            config.as_c_ptr(),
                            cpath.as_ptr(),
                            ffi::GIT_CONFIG_LEVEL_LOCAL,
                            ptr::null(),
                            0,
                        )
                    };
                    assert_eq!(rc, 0);
                    // A first error allocates the thread's error state, which an
                    // allocation failure would otherwise report instead.
                    drop(Config::new().unwrap().as_mut().lock().unwrap_err());
                    REFUSING.store(true, SeqCst);
                    // The pool's page is refused. A refused refdb allocation instead
                    // crashes in C (`git_refdb_open` frees an uninitialised pointer), see
                    // `Transaction::new`.
                    let tx = Transaction::new(repo.as_ref()).map(drop);
                    let locked = config.as_mut().lock().map(drop);
                    REFUSING.store(false, SeqCst);
                    assert_eq!(tx.unwrap_err().class(), Some(ErrorClass::NoMemory));
                    let locked = locked.unwrap_err();
                    assert_eq!(locked.code(), Some(ErrorCode::Generic));
                    assert_eq!(locked.class(), Some(ErrorClass::NoMemory));
                    assert!(!path.with_extension("lock").exists());
                    drop(Transaction::new(repo.as_ref()).unwrap());
                    drop(config.as_mut().lock().unwrap());
                    assert!(!path.with_extension("lock").exists());
                    drop(config);
                    drop(std::fs::remove_file(path));
                },
            );
        }
    }
}
