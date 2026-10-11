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
use crate::libgit2::config::config::{ConfigMut, ConfigRef};
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

/// Opaque reference transaction, borrowing its repository for `'r`.
///
/// The refdb reference C retains does not keep that repository alive. Owners and
/// handles therefore cannot escape the repository's borrow:
///
/// ```compile_fail
/// use libgit2::libgit2::refs::transaction::{Transaction, TransactionBox};
/// use libgit2::libgit2::repository::repository::Repository;
/// fn escape() -> TransactionBox<'static> {
///     let repo = Repository::open(c".").unwrap();
///     Transaction::new(repo.as_ref()).unwrap()
/// }
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::refs::transaction::TransactionMut;
/// fn shrink<'a, 'r>(m: TransactionMut<'a, 'static>) -> TransactionMut<'a, 'r> { m }
/// ```
///
/// Neither an owner nor a shared owner crosses threads:
///
/// ```compile_fail
/// use libgit2::libgit2::refs::transaction::TransactionBox;
/// fn require_send<T: Send>() {}
/// require_send::<TransactionBox<'static>>();
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::refs::transaction::TransactionBox;
/// fn require_sync<T: Sync>() {}
/// require_sync::<TransactionBox<'static>>();
/// ```
///
/// Wraps: git_transaction
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
    type Ref<'a>
        = TransactionRef<'a, 'r>
    where
        Self: 'a;
    type Mut<'a>
        = TransactionMut<'a, 'r>
    where
        Self: 'a;
}

impl TransactionRef<'_, '_> {
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_transaction {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> TransactionMut<'_, 'r> {
    #[allow(dead_code)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_transaction {
        self.0.0.as_non_null().as_ptr().cast()
    }

    pub fn as_ref(&self) -> TransactionRef<'_, 'r> {
        self.0
    }

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
/// Wraps: git_transaction_free
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
    /// Wraps: git_transaction_new
    pub fn new(repo: RepositoryRef<'r>) -> Result<TransactionBox<'r>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: out is writable and repo is live for 'r; C stores that borrow
        // and obtains its own refdb reference. On failure it cleans its pool and
        // leaves out untouched. The runtime remains initialized through teardown.
        let rc = unsafe { ffi::git_transaction_new(&mut out, repo.as_ptr().cast_mut()) };
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
/// use libgit2::libgit2::config::config::Config;
/// let mut config = Config::new().unwrap();
/// let tx = config.as_mut().lock().unwrap();
/// drop(config);
/// drop(tx);
/// ```
///
/// ```compile_fail
/// use libgit2::libgit2::config::config::Config;
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
    transaction: TransactionBox<'c>,
    config: ConfigMut<'c, 'r>,
}

impl<'c, 'r> ConfigTransaction<'c, 'r> {
    /// Wraps: git_transaction_config_new
    pub(crate) fn lock(mut config: ConfigMut<'c, 'r>) -> Result<Self, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: config grants exclusive access for 'c; out is writable. C locks
        // the writer backend and retains one reference to it on success. The
        // returned owner keeps config live through unlock, and never treats this
        // config transaction as a reference transaction.
        let rc = unsafe { ffi::git_config_lock(&mut out, config.as_mut_ptr()) };
        if rc < 0 {
            return Err(last());
        }
        // SAFETY: success transfers a fresh config transaction; TransactionFree
        // dispatches its release correctly. The stored ConfigMut keeps cfg alive.
        let transaction = unsafe { TransactionBox::from_c(out) }.ok_or_else(last)?;
        Ok(Self {
            transaction,
            config,
        })
    }

    pub fn config(&self) -> ConfigRef<'_, 'r> {
        self.config.as_ref()
    }

    #[allow(dead_code)]
    pub(crate) fn config_mut(&mut self) -> ConfigMut<'_, 'r> {
        self.config.as_mut()
    }

    // The later commit-symbol wrapper must consume this owner: C clears cfg and
    // cfg_data after commit, so committing a config transaction twice is invalid.
    #[allow(dead_code)]
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
        assert_eq!(size_of::<Transaction>(), size_of::<ffi::git_transaction>());
        assert_eq!(
            align_of::<Transaction>(),
            align_of::<ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<TransactionBox>(),
            size_of::<*mut ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<Option<TransactionBox>>(),
            size_of::<*mut ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<TransactionRef>(),
            size_of::<*mut ffi::git_transaction>()
        );
        assert_eq!(
            size_of::<TransactionMut>(),
            size_of::<*mut ffi::git_transaction>()
        );
        // SAFETY: bindgen represents this opaque type as a zero-sized byte array,
        // for which zero is valid. It is never passed to C or read as a C body.
        let mut raw = unsafe { core::mem::zeroed::<ffi::git_transaction>() };
        let p = NonNull::from(&mut raw).cast::<Transaction>();
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
        use crate::libgit2::config::config::Config;
        let path =
            std::env::temp_dir().join(format!("crustify-unit-tx-config-{}", std::process::id()));
        let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        let _ = std::fs::remove_file(&path);
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
            assert_eq!(
                ffi::git_config_set_int32(config.as_c_ptr(), c"a.b".as_ptr(), 1),
                0
            );
        }
        let mut tx = config.as_mut().lock().unwrap();
        // SAFETY: the exclusive reborrow keeps the config alive and the setter
        // stages a scalar copy. Commit is performed once, before dropping tx.
        unsafe {
            assert_eq!(
                ffi::git_config_set_int32(tx.config_mut().as_mut_ptr(), c"a.b".as_ptr(), 2),
                0
            );
            assert_eq!(ffi::git_transaction_commit(tx.as_mut_ptr()), 0);
        }
        drop(tx); // Must not unlock again after C cleared cfg/cfg_data.
        assert!(std::fs::read_to_string(&path).unwrap().contains("2"));
        drop(config);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adoption_and_pathless_construction() {
        use crate::libgit2::repository::repository::Repository;
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
}
