//! Safe wrappers over `include/git2/errors.h`.
//!
//! libgit2 publishes a `git_error` only by `const` pointer, from `git_error_last`,
//! and never hands one over: it is either one of four static errors (out of memory,
//! not initialised, thread-state failure, no error) or the calling thread's error
//! state, whose `message` aliases a buffer that the next `git_error_set*` or
//! `git_error_clear` on that thread rewrites and may reallocate. Its lifecycle routines
//! (`git_error_save`, `git_error_restore`, `git_error_free`) are internal to
//! libgit2.so.
//!
//! So an [`ErrorRef`] is valid only until the next libgit2 call on its thread, which no
//! Rust lifetime can express across safe calls: whoever builds one must not hand it out
//! past such a call. [`ErrorBuf`] is the owned copy that outlives it, and crosses
//! threads; it is what a safe wrapper returns, [`last`] among them.
//!
//! libgit2 never names its error enums in a signature: functions return a
//! `git_error_code` as a plain `int`, `git_error.klass` holds a `git_error_t` as one,
//! and `git_error_set` / `git_error_set_str` take the class as one. Callbacks may
//! return any `int`, which libgit2 hands back unchanged, and user code may set any
//! class, so a raw value is converted fallibly ([`ErrorCode::from_raw`],
//! [`ErrorClass::from_raw`]) and never transmuted.
//!
//! Safe wrappers over `include/git2/sys/errors.h`: setting and clearing the calling
//! thread's last libgit2 error.
//!
//! libgit2 keeps one last error per thread (`errors.c`'s `error_threadstate`), in
//! thread-local storage under a key that `git_libgit2_init` creates and the last
//! `git_libgit2_shutdown` deletes. Every routine here looks that state up first;
//! without a live key the lookup reads whatever block TLS key 0 holds, and writes into
//! it. Each wrapper therefore pins the runtime first (`runtime_pinned`), and does
//! nothing when libgit2 cannot be initialised. C callers reaching these routines
//! before initialisation corrupt foreign thread-local state instead; the wrappers
//! deliberately diverge from it.
//!
//! The thread's message lives in one buffer that every later error overwrites, whether
//! set here or by any other libgit2 call on the thread, so the message `git_error_last`
//! reports must be copied out before the next one.
//!
//! Once an allocation for a message fails, that buffer stays out of memory for the
//! rest of the thread (`str.c` marks it `git_str__oom`, which `git_str_clear` keeps):
//! the failure leaves `GIT_ERROR_NOMEMORY` as the last error, and every later message
//! on the thread is dropped — [`set`] and [`set_without_message`] keep the previous
//! error, [`set_str`] fails. The wrappers keep libgit2's behaviour.
//!
//! The error class is an [`ErrorClass`], which libgit2 stores as the `int` its
//! `git_error_t` constant is. C code may store an `int` the header does not name,
//! which `git_error_last` reports as a raw class; the setters here only take named ones.

use core::ffi::{CStr, c_int};
use core::{fmt, ptr};
use std::ffi::CString;
use std::io;
use std::sync::OnceLock;

use ffibox::{CRefSeam, define_ctype};
use glibc_bindings::libc::csu::errno_loc::set_errno;

use crate::ffi;
use crate::libgit2::src::libgit2::libgit2::{Runtime, init};

/// Declares a fieldless enum over a C enum's bindgen constants: each variant's
/// discriminant is its constant, `from_raw` maps an `int` back to the variant
/// carrying it, and `raw` gives the `int` C traffics in. The second item is the
/// error `TryFrom<c_int>` reports, carrying the `int` the enum does not name.
macro_rules! c_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($(#[$vmeta:meta])* $variant:ident = $c:expr,)*
        }
        $(#[$umeta:meta])*
        pub struct $unnamed:ident($what:literal);
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(i32)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$vmeta])* $variant = $c,)*
        }

        impl $name {
            /// The variant whose C value is `raw`; `None` for a value this version of
            /// libgit2's header does not name.
            pub const fn from_raw(raw: c_int) -> Option<Self> {
                $(if raw == Self::$variant as c_int {
                    return Some(Self::$variant);
                })*
                None
            }

            /// The C value, as libgit2's functions take and return it.
            pub const fn raw(self) -> c_int {
                self as c_int
            }
        }

        impl TryFrom<c_int> for $name {
            type Error = $unnamed;

            fn try_from(raw: c_int) -> Result<Self, $unnamed> {
                Self::from_raw(raw).ok_or($unnamed(raw))
            }
        }

        impl From<$name> for c_int {
            fn from(value: $name) -> c_int {
                value.raw()
            }
        }

        $(#[$umeta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $unnamed(c_int);

        impl $unnamed {
            /// The value, as libgit2 produced it.
            pub const fn raw(self) -> c_int {
                self.0
            }
        }

        impl fmt::Display for $unnamed {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!("libgit2 ", $what, " {} is not named by git2/errors.h"), self.0)
            }
        }

        impl std::error::Error for $unnamed {}
    };
}

c_enum! {
    /// A generic return code of libgit2's functions.
    ///
    /// [`Ok`](Self::Ok) is success; every other variant is negative. A callback's own
    /// return value can surface in its place, and need not be a variant: a positive
    /// value, or a negative one the header does not name, gives `None` from
    /// [`from_raw`](Self::from_raw).
    ///
    /// Wraps: `git_error_code`
    /// Handwritten
    pub enum ErrorCode {
        /// No error occurred; the call was successful.
        Ok = ffi::GIT_OK,
        /// An error occurred; `git_error_last` has more information.
        Generic = ffi::GIT_ERROR,
        /// Requested object could not be found.
        NotFound = ffi::GIT_ENOTFOUND,
        /// Object exists preventing operation.
        Exists = ffi::GIT_EEXISTS,
        /// More than one object matches.
        Ambiguous = ffi::GIT_EAMBIGUOUS,
        /// Output buffer too short to hold data.
        BufSize = ffi::GIT_EBUFS,
        /// Never generated by libgit2: a callback returns it (to stop an iteration,
        /// say) to know that the error came from the callback.
        User = ffi::GIT_EUSER,
        /// Operation not allowed on bare repository.
        BareRepo = ffi::GIT_EBAREREPO,
        /// HEAD refers to branch with no commits.
        UnbornBranch = ffi::GIT_EUNBORNBRANCH,
        /// Merge in progress prevented operation.
        Unmerged = ffi::GIT_EUNMERGED,
        /// Reference was not fast-forwardable.
        NonFastForward = ffi::GIT_ENONFASTFORWARD,
        /// Name/ref spec was not in a valid format.
        InvalidSpec = ffi::GIT_EINVALIDSPEC,
        /// Checkout conflicts prevented operation.
        Conflict = ffi::GIT_ECONFLICT,
        /// Lock file prevented operation.
        Locked = ffi::GIT_ELOCKED,
        /// Reference value does not match expected.
        Modified = ffi::GIT_EMODIFIED,
        /// Authentication error.
        Auth = ffi::GIT_EAUTH,
        /// Server certificate is invalid.
        Certificate = ffi::GIT_ECERTIFICATE,
        /// Patch/merge has already been applied.
        Applied = ffi::GIT_EAPPLIED,
        /// The requested peel operation is not possible.
        Peel = ffi::GIT_EPEEL,
        /// Unexpected EOF.
        Eof = ffi::GIT_EEOF,
        /// Invalid operation or input.
        Invalid = ffi::GIT_EINVALID,
        /// Uncommitted changes in index prevented operation.
        Uncommitted = ffi::GIT_EUNCOMMITTED,
        /// The operation is not valid for a directory.
        Directory = ffi::GIT_EDIRECTORY,
        /// A merge conflict exists and cannot continue.
        MergeConflict = ffi::GIT_EMERGECONFLICT,
        /// A user-configured callback refused to act.
        Passthrough = ffi::GIT_PASSTHROUGH,
        /// Signals end of iteration with iterator.
        IterOver = ffi::GIT_ITEROVER,
        /// Internal only.
        Retry = ffi::GIT_RETRY,
        /// Hashsum mismatch in object.
        Mismatch = ffi::GIT_EMISMATCH,
        /// Unsaved changes in the index would be overwritten.
        IndexDirty = ffi::GIT_EINDEXDIRTY,
        /// Patch application failed.
        ApplyFail = ffi::GIT_EAPPLYFAIL,
        /// The object is not owned by the current user.
        Owner = ffi::GIT_EOWNER,
        /// The operation timed out.
        Timeout = ffi::GIT_TIMEOUT,
        /// There were no changes.
        Unchanged = ffi::GIT_EUNCHANGED,
        /// An option is not supported.
        NotSupported = ffi::GIT_ENOTSUPPORTED,
        /// The subject is read-only.
        ReadOnly = ffi::GIT_EREADONLY,
    }

    /// A return code that this version of libgit2's header does not name: a callback's
    /// own result handed back, or a code of a later header. [`raw`](Self::raw) has it.
    ///
    /// Handwritten
    pub struct UnnamedErrorCode("error code");
}

c_enum! {
    /// The category of an error: the area of the code where it occurred.
    ///
    /// `git_error.klass` and the class argument of `git_error_set` carry it as an
    /// `int`, which user code may set to a value the header does not name; such a
    /// value gives `None` from [`from_raw`](Self::from_raw).
    ///
    /// Wraps: `git_error_t`
    /// Handwritten
    pub enum ErrorClass {
        /// No error.
        None = ffi::GIT_ERROR_NONE.cast_signed(),
        /// Out of memory.
        NoMemory = ffi::GIT_ERROR_NOMEMORY.cast_signed(),
        /// An operating-system call failed.
        Os = ffi::GIT_ERROR_OS.cast_signed(),
        /// Invalid input or operation.
        Invalid = ffi::GIT_ERROR_INVALID.cast_signed(),
        /// References.
        Reference = ffi::GIT_ERROR_REFERENCE.cast_signed(),
        /// zlib compression.
        Zlib = ffi::GIT_ERROR_ZLIB.cast_signed(),
        /// Repositories.
        Repository = ffi::GIT_ERROR_REPOSITORY.cast_signed(),
        /// Configuration.
        Config = ffi::GIT_ERROR_CONFIG.cast_signed(),
        /// Regular expressions.
        Regex = ffi::GIT_ERROR_REGEX.cast_signed(),
        /// The object database.
        Odb = ffi::GIT_ERROR_ODB.cast_signed(),
        /// The index.
        Index = ffi::GIT_ERROR_INDEX.cast_signed(),
        /// Objects.
        Object = ffi::GIT_ERROR_OBJECT.cast_signed(),
        /// Networking.
        Net = ffi::GIT_ERROR_NET.cast_signed(),
        /// Tags.
        Tag = ffi::GIT_ERROR_TAG.cast_signed(),
        /// Trees.
        Tree = ffi::GIT_ERROR_TREE.cast_signed(),
        /// The pack indexer.
        Indexer = ffi::GIT_ERROR_INDEXER.cast_signed(),
        /// TLS.
        Ssl = ffi::GIT_ERROR_SSL.cast_signed(),
        /// Submodules.
        Submodule = ffi::GIT_ERROR_SUBMODULE.cast_signed(),
        /// Threading.
        Thread = ffi::GIT_ERROR_THREAD.cast_signed(),
        /// Stashes.
        Stash = ffi::GIT_ERROR_STASH.cast_signed(),
        /// Checkout.
        Checkout = ffi::GIT_ERROR_CHECKOUT.cast_signed(),
        /// `FETCH_HEAD`.
        FetchHead = ffi::GIT_ERROR_FETCHHEAD.cast_signed(),
        /// Merges.
        Merge = ffi::GIT_ERROR_MERGE.cast_signed(),
        /// SSH.
        Ssh = ffi::GIT_ERROR_SSH.cast_signed(),
        /// Filters.
        Filter = ffi::GIT_ERROR_FILTER.cast_signed(),
        /// Reverts.
        Revert = ffi::GIT_ERROR_REVERT.cast_signed(),
        /// A user callback.
        Callback = ffi::GIT_ERROR_CALLBACK.cast_signed(),
        /// Cherry-picks.
        CherryPick = ffi::GIT_ERROR_CHERRYPICK.cast_signed(),
        /// `git describe`.
        Describe = ffi::GIT_ERROR_DESCRIBE.cast_signed(),
        /// Rebases.
        Rebase = ffi::GIT_ERROR_REBASE.cast_signed(),
        /// The filesystem.
        Filesystem = ffi::GIT_ERROR_FILESYSTEM.cast_signed(),
        /// Patches.
        Patch = ffi::GIT_ERROR_PATCH.cast_signed(),
        /// Worktrees.
        Worktree = ffi::GIT_ERROR_WORKTREE.cast_signed(),
        /// Hashing.
        Sha = ffi::GIT_ERROR_SHA.cast_signed(),
        /// HTTP.
        Http = ffi::GIT_ERROR_HTTP.cast_signed(),
        /// libgit2 itself.
        Internal = ffi::GIT_ERROR_INTERNAL.cast_signed(),
        /// Grafts.
        Grafts = ffi::GIT_ERROR_GRAFTS.cast_signed(),
    }

    /// An error class that this version of libgit2's header does not name, which user
    /// code can set through `git_error_set_str`. [`raw`](Self::raw) has it.
    ///
    /// Handwritten
    pub struct UnnamedErrorClass("error class");
}

/// The class as `git_error_t`, the type of the C constants (the setters in
/// [`sys::errors`](crate::libgit2::src::util::errors) take the [`ErrorClass`] itself).
impl From<ErrorClass> for ffi::git_error_t {
    fn from(class: ErrorClass) -> ffi::git_error_t {
        // Every class is non-negative, so the value is unchanged.
        class.raw().cast_unsigned()
    }
}

define_ctype!(
    /// Extra details of a libgit2 error: its class and message (`git_error`).
    ///
    /// Only ever borrowed: an [`ErrorRef`] over an error libgit2 published, whose
    /// getters read it in place. Besides `from_ptr`'s requirements, building one
    /// requires `message` to be NULL or a NUL-terminated string, both staying live and
    /// unchanged for the handle's lifetime: for a thread's error, until the next
    /// libgit2 call on that thread. The handles are neither `Send` nor `Sync`, since
    /// that error belongs to its thread:
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::src::util::errors::ErrorRef;
    ///
    /// fn send<T: Send>(_: T) {}
    /// fn forward(error: ErrorRef<'static>) {
    ///     send(error);
    /// }
    /// ```
    ///
    /// There are no setters: libgit2 publishes the struct read-only and no public
    /// routine takes one. [`ErrorRef::to_error_buf`] copies it.
    ///
    /// Wraps: git_error
    Error,
    ErrorRef,
    ErrorMut,
    ffi::git_error
);

impl<'a> ErrorRef<'a> {
    /// The error message, borrowed from the error. libgit2 never publishes a NULL
    /// message; one reads as empty.
    ///
    /// Field: `git_error.message`
    #[must_use]
    pub fn message(&self) -> &'a CStr {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let message_ptr = unsafe { &raw const (*self.as_ptr()).message };
        // SAFETY: a read through the shared handle's pointer to a live `git_error`.
        let message = unsafe { message_ptr.read() };
        if message.is_null() {
            return c"";
        }
        // SAFETY: per this handle's invariant, a non-NULL `message` is a NUL-terminated
        // string that stays live and unchanged for `'a`.
        unsafe { CStr::from_ptr(message) }
    }

    /// The error's class; `None` for a class the header does not name, which user
    /// code can set through `git_error_set_str` ([`raw_class`](Self::raw_class) has it).
    ///
    /// Field: `git_error.klass`
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        ErrorClass::from_raw(self.raw_class())
    }

    /// The error's class as libgit2 stores it, whether or not the header names it.
    ///
    /// Field: `git_error.klass`
    #[must_use]
    pub fn raw_class(&self) -> c_int {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let klass_ptr = unsafe { &raw const (*self.as_ptr()).klass };
        // SAFETY: a read through the shared handle's pointer to a live `git_error`.
        unsafe { klass_ptr.read() }
    }

    /// An owned copy of the error, which outlives it.
    #[must_use]
    pub fn to_error_buf(&self) -> ErrorBuf {
        ErrorBuf {
            class: self.raw_class(),
            message: self.message().to_owned(),
        }
    }
}

/// Shows a raw class as its [`ErrorClass`] variant, or as the `int` when the header
/// does not name it.
struct DebugClass(c_int);

impl fmt::Debug for DebugClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match ErrorClass::from_raw(self.0) {
            Some(class) => class.fmt(f),
            None => self.0.fmt(f),
        }
    }
}

impl fmt::Debug for ErrorRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("class", &DebugClass(self.raw_class()))
            .field("message", &self.message())
            .finish()
    }
}

/// An owned copy of a libgit2 error, held in Rust storage.
///
/// libgit2's own copy, `git_error_save`, is internal to libgit2.so, and nothing public
/// takes a `git_error` back, so the copy needs no C layout. It displays as its message;
/// libgit2 messages are bytes (paths among them), shown lossily.
///
/// Handwritten
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ErrorBuf {
    class: c_int,
    message: CString,
}

impl ErrorBuf {
    /// An error of the raw `class` described by `message`, built in Rust.
    pub(crate) fn new(class: c_int, message: &CStr) -> Self {
        Self {
            class,
            message: message.to_owned(),
        }
    }

    /// The error's class; `None` for a class the header does not name
    /// ([`raw_class`](Self::raw_class) has it).
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        ErrorClass::from_raw(self.class)
    }

    /// The error's class as libgit2 stored it, whether or not the header names it.
    #[must_use]
    pub fn raw_class(&self) -> c_int {
        self.class
    }

    /// The error message.
    #[must_use]
    pub fn message(&self) -> &CStr {
        &self.message
    }
}

impl From<ErrorRef<'_>> for ErrorBuf {
    fn from(error: ErrorRef<'_>) -> Self {
        error.to_error_buf()
    }
}

impl fmt::Debug for ErrorBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorBuf")
            .field("class", &DebugClass(self.class))
            .field("message", &self.message)
            .finish()
    }
}

impl fmt::Display for ErrorBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message.to_string_lossy())
    }
}

impl std::error::Error for ErrorBuf {}

/// The calling thread's last libgit2 error, copied out.
///
/// It is the error the last failing libgit2 call on this thread left, or one of
/// libgit2's static errors: `GIT_ERROR_NONE`, "no error", while the thread has none
/// (never set, or cleared); `GIT_ERROR_NOMEMORY`, "Out of memory", after an allocation
/// failure; `GIT_ERROR_THREAD`, "thread-local data initialization failure", when the
/// thread's error state cannot be allocated. A call that succeeds may leave an older
/// error in place, so it describes a failure only right after a call reported one:
/// detect errors by return codes.
///
/// libgit2 lends the error only until the next libgit2 call on this thread, so it is
/// copied: the [`ErrorBuf`] outlives it, and crosses threads.
///
/// Like the setters of [`sys::errors`](crate::libgit2::src::util::errors), it pins libgit2's
/// runtime first, which initialises libgit2 on the first call: unpinned, a last
/// `git_libgit2_shutdown` on another thread could delete the thread-state TLS key between libgit2's
/// check that it is initialised and its lookup under that key. So where C's `git_error_last`,
/// called before initialisation, reports `GIT_ERROR_INVALID`, "library has not been
/// initialized", this one reports the thread's error in the pinned runtime. It returns
/// that static error only when libgit2 cannot be initialised, without calling
/// libgit2: a failed initialisation can leave the thread state without a key.
///
/// Wraps: `git_error_last`
///
/// # Panics
///
/// Panics, with `git_error_last never returns NULL`, only if libgit2 breaks its contract.
#[must_use]
pub fn last() -> ErrorBuf {
    if !runtime_pinned() {
        return ErrorBuf {
            class: ErrorClass::Invalid.raw(),
            message: c"library has not been initialized".to_owned(),
        };
    }
    // SAFETY: no arguments; the runtime is pinned, so the thread state is looked up
    // under libgit2's live TLS key.
    let error = unsafe { ffi::git_error_last() };
    // SAFETY: `error` is NULL or, as libgit2 always returns, a static `git_error` or this
    // thread's error state, either with a NUL-terminated message; both stay live and
    // unchanged until the next libgit2 call on this thread, and the handle is dropped
    // once copied, before any.
    let error = unsafe { ErrorRef::from_ptr(error.cast_mut()) };
    error
        .expect("git_error_last never returns NULL")
        .to_error_buf()
}

/// Holds one `git_libgit2_init` reference for the rest of the process; `false` when
/// libgit2 cannot be initialised.
///
/// Until the first initialisation, and after the last shutdown, libgit2's error state
/// is looked up under a TLS key that does not exist: `errors.c` then writes into
/// whatever block key 0 holds. Its allocator also fails every request, and
/// `git__malloc` reports that through the same state. The reference is a [`Runtime`]
/// that is never dropped, so it keeps the key live across balanced [`init`] /
/// shutdown pairs; a refused one is final, as [`init`] refuses every initialisation
/// after a failed one.
///
/// The calling thread's `errno` survives the first call, which initialises libgit2,
/// so a `GIT_ERROR_OS` error set right after still describes the caller's failure.
pub(crate) fn runtime_pinned() -> bool {
    static PINNED: OnceLock<Option<Runtime>> = OnceLock::new();
    if let Some(pinned) = PINNED.get() {
        return pinned.is_some();
    }
    let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
    let pinned = PINNED.get_or_init(|| init().ok()).is_some();
    set_errno(errno);
    pinned
}

/// Clears the calling thread's last libgit2 error: `git_error_last` then reports
/// `GIT_ERROR_NONE`, "no error".
///
/// Also resets the thread's `errno` to 0, as libgit2 does, unless the thread's error
/// state cannot be allocated.
///
/// Wraps: `git_error_clear`
pub fn clear() {
    if runtime_pinned() {
        // SAFETY: no arguments; the runtime is pinned, so the thread state is looked up
        // under libgit2's live TLS key.
        unsafe { ffi::git_error_clear() }
    }
}

/// Sets the calling thread's last error to `message` in `class`.
///
/// `message` is handed to `git_error_set` as the argument of a `"%s"` format, so a `%`
/// in it is literal. For [`ErrorClass::Os`], libgit2 appends `": "` and, when the
/// thread's `errno` is non-zero, its description, then resets `errno` to 0. libgit2
/// copies the message; nothing borrows it after the call.
///
/// When the message cannot be stored, the last error is `GIT_ERROR_NOMEMORY`, or the
/// previous one once the thread's buffer is out of memory (see the module docs). A
/// message of `c_int::MAX` bytes or more cannot be formatted at all: libgit2 then
/// keeps the previous error and the thread's buffer goes out of memory for good;
/// [`set_str`] has no such limit.
///
/// ```compile_fail,E0308
/// // The message must be NUL-terminated: a `&str` is not.
/// use libgit2::libgit2::src::util::errors::ErrorClass;
/// libgit2::libgit2::src::util::errors::set(ErrorClass::Invalid, "bad");
/// ```
///
/// ```compile_fail,E0308
/// // The class is a named one, not the `int` libgit2 stores.
/// libgit2::libgit2::src::util::errors::set(libgit2::ffi::GIT_ERROR_INVALID, c"bad");
/// ```
///
/// Wraps: `git_error_set`
pub fn set(class: ErrorClass, message: &CStr) {
    if runtime_pinned() {
        // SAFETY: a `"%s"` format with the one NUL-terminated string argument it
        // names, both read during the call only; `class` is the `int` of a
        // `git_error_t` constant. The runtime is pinned, so the thread state is looked
        // up under libgit2's live TLS key.
        unsafe { ffi::git_error_set(class.raw(), c"%s".as_ptr(), message.as_ptr()) }
    }
}

/// Sets the calling thread's last error to `class` without a message of its own:
/// `git_error_set` with a NULL format.
///
/// For [`ErrorClass::Os`] the message is the description of the thread's `errno` when
/// it is non-zero, and empty otherwise, and `errno` is reset to 0; for any other class
/// the message is empty.
///
/// Only the `errno` description needs storage: when it cannot be stored, the last
/// error is `GIT_ERROR_NOMEMORY`. Once the thread's buffer is out of memory, the
/// previous error stays, whatever the class (see the module docs).
///
/// Wraps: `git_error_set`
pub fn set_without_message(class: ErrorClass) {
    if runtime_pinned() {
        // SAFETY: a NULL format, which libgit2 checks before formatting, so no variadic
        // argument is read; `class` is the `int` of a `git_error_t` constant. The
        // runtime is pinned.
        unsafe { ffi::git_error_set(class.raw(), ptr::null()) }
    }
}

/// Sets the calling thread's last error to a verbatim copy of `message` in `class`.
///
/// Unlike [`set`], it never consults `errno`, and it reports whether the message was stored.
///
/// Wraps: `git_error_set_str`
///
/// # Errors
///
/// On failure the last error is `GIT_ERROR_NOMEMORY`, or the previous one when the thread's buffer
/// is already out of memory (see the module docs).
pub fn set_str(class: ErrorClass, message: &CStr) -> Result<(), MessageNotStored> {
    if !runtime_pinned() {
        return Err(MessageNotStored);
    }
    // SAFETY: a NUL-terminated string, copied during the call and never retained;
    // `class` is the `int` of a `git_error_t` constant. The runtime is pinned.
    match unsafe { ffi::git_error_set_str(class.raw(), message.as_ptr()) } {
        0 => Ok(()),
        _ => Err(MessageNotStored),
    }
}

/// Sets the calling thread's last error to libgit2's static out-of-memory error,
/// `GIT_ERROR_NOMEMORY`, "Out of memory", copying no message.
///
/// Use it to report an allocation failure: unlike [`set_str`] it needs no storage for
/// the message, beyond the thread's error state on its first use.
///
/// Wraps: `git_error_set_oom`
pub fn set_oom() {
    if runtime_pinned() {
        // SAFETY: no arguments; the runtime is pinned, so the thread state is looked up
        // under libgit2's live TLS key.
        unsafe { ffi::git_error_set_oom() }
    }
}

/// [`set_str`] could not store the message: libgit2 ran out of memory, or cannot be
/// initialised.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageNotStored;

impl fmt::Display for MessageNotStored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("libgit2 could not store the error message")
    }
}

impl std::error::Error for MessageNotStored {}

#[cfg(test)]
mod unit_tests {
    use core::ptr::NonNull;
    use std::process::Command;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering::SeqCst;

    use super::*;
    use crate::libgit2::src::util::alloc::{Alloc, Allocator};

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

    fn init() {
        // SAFETY: no arguments; the reference is never released, so the runtime and
        // this process's thread states stay live.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// `git_error_last()` adopted as a handle, and C's own reading of it.
    ///
    /// The handle is used only before the next libgit2 call on this thread.
    fn last() -> (ErrorRef<'static>, Vec<u8>, c_int) {
        {
            // SAFETY: `git_error_last` never returns NULL; the error and its message stay
            // live and unchanged until the next libgit2 call on this thread, which every
            // caller makes only after it is done with the handle.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above.
            let message2 = unsafe { (*e).message };
            // SAFETY: as above.
            let c_message = unsafe { CStr::from_ptr(message2).to_bytes().to_vec() };
            // SAFETY: as above.
            let value = unsafe { ErrorRef::from_ptr(e.cast_mut()) };
            // SAFETY: as above.
            unsafe { (value.unwrap(), c_message, (*e).klass) }
        }
    }

    fn set_str(class: ffi::git_error_t, message: &CStr) {
        // SAFETY: `message` is a live C string, which libgit2 copies.
        let rc = unsafe { ffi::git_error_set_str(class.cast_signed(), message.as_ptr()) };
        assert_eq!(rc, 0);
    }

    #[test]
    fn null_message_reads_as_empty() {
        let mut raw = ffi::git_error {
            message: ptr::null_mut(),
            klass: 3,
        };
        // SAFETY: `raw` is a live `git_error` with a NULL message for the handle's life.
        let e = unsafe { ErrorRef::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!(e.message(), c"");
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        assert_eq!(e.raw_class(), 3);
        assert_eq!(e.to_error_buf().message(), c"");
    }

    #[test]
    fn copy_matches_and_outlives_the_borrowed_error() {
        let mut text = *b"bad object\0";
        let mut raw = ffi::git_error {
            message: text.as_mut_ptr().cast(),
            klass: 9,
        };
        let buf = {
            // SAFETY: `raw` and `text` stay live and unchanged for the handle's life.
            let e = unsafe { ErrorRef::from_ptr(&raw mut raw) }.unwrap();
            assert_eq!(e.message(), c"bad object");
            assert_eq!(e.class(), Some(ErrorClass::Odb));
            assert_eq!(
                format!("{e:?}"),
                r#"Error { class: Odb, message: "bad object" }"#
            );
            ErrorBuf::from(e)
        };
        text[0] = b'X';
        assert_eq!(&text[..3], b"Xad");
        assert_eq!(buf.message(), c"bad object");
        assert_eq!(buf.class(), Some(ErrorClass::Odb));
        assert_eq!(buf.raw_class(), 9);
        assert_eq!(
            format!("{buf:?}"),
            r#"ErrorBuf { class: Odb, message: "bad object" }"#
        );
        assert_eq!(buf.to_string(), "bad object");
        assert_eq!(buf.clone(), buf);
        let dyn_err: &dyn std::error::Error = &buf;
        assert!(dyn_err.source().is_none());
    }

    #[test]
    fn non_utf8_message_displays_lossily() {
        let mut text = *b"path \xff\0";
        let mut raw = ffi::git_error {
            message: text.as_mut_ptr().cast(),
            klass: 2,
        };
        // SAFETY: as above.
        let e = unsafe { ErrorRef::from_ptr(&raw mut raw) }.unwrap();
        let buf = e.to_error_buf();
        assert_eq!(buf.message().to_bytes(), b"path \xff");
        assert_eq!(buf.to_string(), "path \u{fffd}");
    }

    #[test]
    fn unnamed_class_is_kept_raw() {
        let mut text = *b"custom\0";
        let mut raw = ffi::git_error {
            message: text.as_mut_ptr().cast(),
            klass: 1000,
        };
        // SAFETY: as above.
        let e = unsafe { ErrorRef::from_ptr(&raw mut raw) }.unwrap();
        assert_eq!((e.class(), e.raw_class()), (None, 1000));
        assert_eq!(
            format!("{e:?}"),
            r#"Error { class: 1000, message: "custom" }"#
        );
        let buf = e.to_error_buf();
        assert_eq!((buf.class(), buf.raw_class()), (None, 1000));
        assert_eq!(
            format!("{buf:?}"),
            r#"ErrorBuf { class: 1000, message: "custom" }"#
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn thread_error_reads_as_c_does() {
        init();
        set_str(ffi::GIT_ERROR_CONFIG, c"first failure");
        let (e, message, klass) = last();
        assert_eq!(e.message().to_bytes(), message);
        assert_eq!(e.raw_class(), klass);
        assert_eq!(e.class(), Some(ErrorClass::Config));
        assert_eq!(e.message(), c"first failure");
    }

    /// The copy survives the thread's message buffer being rewritten and reallocated,
    /// which leaves a borrowed message dangling; `ASan` flags a copy that aliases it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copy_survives_the_next_error() {
        init();
        set_str(ffi::GIT_ERROR_ODB, c"short");
        let first = last().0.to_error_buf();
        let long = CString::new("x".repeat(8192)).unwrap();
        set_str(ffi::GIT_ERROR_INDEX, &long);
        let (e, message, klass) = last();
        assert_eq!(e.message(), long.as_c_str());
        assert_eq!(
            (e.message().to_bytes(), e.raw_class()),
            (&message[..], klass)
        );
        assert_eq!(first.message(), c"short");
        assert_eq!(first.class(), Some(ErrorClass::Odb));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn static_errors_read_as_c_does() {
        init();
        // SAFETY: no arguments.
        unsafe { ffi::git_error_clear() };
        let (e, message, klass) = last();
        assert_eq!(
            (e.message(), e.class()),
            (c"no error", Some(ErrorClass::None))
        );
        assert_eq!(
            (e.message().to_bytes(), e.raw_class()),
            (&message[..], klass)
        );

        // SAFETY: no arguments.
        unsafe { ffi::git_error_set_oom() };
        let (e, message, klass) = last();
        assert_eq!(
            (e.message(), e.class()),
            (c"Out of memory", Some(ErrorClass::NoMemory))
        );
        assert_eq!(
            (e.message().to_bytes(), e.raw_class()),
            (&message[..], klass)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn error_before_init_is_static() {
        isolated(
            concat!(module_path!(), "::error_before_init_is_static"),
            || {
                let (e, message, klass) = last();
                assert_eq!(e.message(), c"library has not been initialized");
                assert_eq!(e.class(), Some(ErrorClass::Invalid));
                assert_eq!(
                    (e.message().to_bytes(), e.raw_class()),
                    (&message[..], klass)
                );
            },
        );
    }

    static REFUSING: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut core::ffi::c_void;
        fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
        fn free(p: *mut core::ffi::c_void);
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

    /// Installs [`Refusing`] as libgit2's allocator, before anything is allocated.
    fn install_refusing() {
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

    /// A thread whose error state cannot be allocated reads a static error.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn thread_state_failure_is_static() {
        isolated(
            concat!(module_path!(), "::thread_state_failure_is_static"),
            || {
                install_refusing();
                init();
                REFUSING.store(true, SeqCst);
                let buf = std::thread::spawn(|| {
                    let (e, message, klass) = last();
                    assert_eq!(
                        (e.message().to_bytes(), e.raw_class()),
                        (&message[..], klass)
                    );
                    e.to_error_buf()
                })
                .join()
                .unwrap();
                REFUSING.store(false, SeqCst);
                assert_eq!(buf.message(), c"thread-local data initialization failure");
                assert_eq!(buf.class(), Some(ErrorClass::Thread));
            },
        );
    }

    /// The first `last` initialises libgit2: where C reports its static "not
    /// initialized" error (`error_before_init_is_static`), it reports the thread's empty
    /// state, and C agrees from then on.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn last_before_init_pins_the_runtime() {
        isolated(
            concat!(module_path!(), "::last_before_init_pins_the_runtime"),
            || {
                let buf = super::last();
                assert_eq!(buf.class(), Some(ErrorClass::None));
                assert_eq!(buf.message(), c"no error");
                assert!(runtime_pinned());
                let (e, message, klass) = last();
                assert_eq!(
                    (e.message(), e.class()),
                    (c"no error", Some(ErrorClass::None))
                );
                assert_eq!((&message[..], klass), (b"no error".as_slice(), 0));
            },
        );
    }

    /// Without a runtime, `last` reports C's static "not initialized" error and leaves
    /// libgit2's thread state alone.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn last_without_runtime_is_uninitialised() {
        isolated(
            concat!(module_path!(), "::last_without_runtime_is_uninitialised"),
            || {
                install_refusing();
                REFUSING.store(true, SeqCst);
                let buf = super::last();
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                assert_eq!(buf.class(), Some(ErrorClass::Invalid));
                assert_eq!(buf.raw_class(), ffi::GIT_ERROR_INVALID.cast_signed());
                assert_eq!(buf.message(), c"library has not been initialized");
                assert_eq!(super::last(), buf);
            },
        );
    }

    /// Each thread has its own error; the copy crosses threads, the handle cannot.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copy_crosses_threads() {
        init();
        set_str(ffi::GIT_ERROR_NET, c"here");
        let here = last().0.to_error_buf();
        let there = std::thread::spawn(move || {
            assert_eq!(here.message(), c"here");
            last().0.to_error_buf()
        })
        .join()
        .unwrap();
        assert_eq!(there.message(), c"no error");
        assert_eq!(last().0.message(), c"here");
    }

    const CODES: [ErrorCode; 35] = [
        ErrorCode::Ok,
        ErrorCode::Generic,
        ErrorCode::NotFound,
        ErrorCode::Exists,
        ErrorCode::Ambiguous,
        ErrorCode::BufSize,
        ErrorCode::User,
        ErrorCode::BareRepo,
        ErrorCode::UnbornBranch,
        ErrorCode::Unmerged,
        ErrorCode::NonFastForward,
        ErrorCode::InvalidSpec,
        ErrorCode::Conflict,
        ErrorCode::Locked,
        ErrorCode::Modified,
        ErrorCode::Auth,
        ErrorCode::Certificate,
        ErrorCode::Applied,
        ErrorCode::Peel,
        ErrorCode::Eof,
        ErrorCode::Invalid,
        ErrorCode::Uncommitted,
        ErrorCode::Directory,
        ErrorCode::MergeConflict,
        ErrorCode::Passthrough,
        ErrorCode::IterOver,
        ErrorCode::Retry,
        ErrorCode::Mismatch,
        ErrorCode::IndexDirty,
        ErrorCode::ApplyFail,
        ErrorCode::Owner,
        ErrorCode::Timeout,
        ErrorCode::Unchanged,
        ErrorCode::NotSupported,
        ErrorCode::ReadOnly,
    ];

    #[test]
    fn every_code_round_trips_through_from_raw() {
        for code in CODES {
            assert_eq!(ErrorCode::from_raw(code.raw()), Some(code));
            assert_eq!(ErrorCode::try_from(c_int::from(code)), Ok(code));
        }
    }

    #[test]
    fn codes_have_distinct_values() {
        let mut raw: Vec<c_int> = CODES.iter().map(|c| c.raw()).collect();
        raw.sort_unstable();
        raw.dedup();
        assert_eq!(raw.len(), CODES.len());
    }

    #[test]
    fn unnamed_codes_are_rejected() {
        // The header's gaps, positive callback results and the extremes.
        for raw in [
            -2,
            -25,
            -26,
            -27,
            -28,
            -29,
            -41,
            1,
            7,
            c_int::MIN,
            c_int::MAX,
        ] {
            assert_eq!(ErrorCode::from_raw(raw), None, "{raw}");
            assert_eq!(ErrorCode::try_from(raw), Err(UnnamedErrorCode(raw)));
        }
    }

    #[test]
    fn classes_are_contiguous_from_none_to_grafts() {
        assert_eq!(ErrorClass::None.raw(), 0);
        assert_eq!(ErrorClass::Grafts.raw(), 36);
        for raw in 0..=36 {
            let class = ErrorClass::from_raw(raw).unwrap();
            assert_eq!(class.raw(), raw);
            assert_eq!(c_int::from(class), raw);
            assert_eq!(ErrorClass::try_from(raw), Ok(class));
        }
    }

    #[test]
    fn classes_convert_to_git_error_t_unchanged() {
        assert_eq!(
            ffi::git_error_t::from(ErrorClass::None),
            ffi::GIT_ERROR_NONE
        );
        assert_eq!(ffi::git_error_t::from(ErrorClass::Os), ffi::GIT_ERROR_OS);
        assert_eq!(
            ffi::git_error_t::from(ErrorClass::Grafts),
            ffi::GIT_ERROR_GRAFTS
        );
    }

    #[test]
    fn unnamed_classes_are_rejected() {
        for raw in [-1, 37, 1000, c_int::MIN, c_int::MAX] {
            assert_eq!(ErrorClass::from_raw(raw), None, "{raw}");
            assert_eq!(ErrorClass::try_from(raw), Err(UnnamedErrorClass(raw)));
        }
    }

    /// The rejected value is a well-behaved error: it keeps the `int`, displays it,
    /// and converts into a boxed error that crosses threads.
    #[test]
    fn unnamed_values_are_errors() {
        let code = ErrorCode::try_from(-2).unwrap_err();
        assert_eq!(code.raw(), -2);
        assert_eq!(
            code.to_string(),
            "libgit2 error code -2 is not named by git2/errors.h"
        );
        assert_eq!(format!("{code:?}"), "UnnamedErrorCode(-2)");
        let class = ErrorClass::try_from(1000).unwrap_err();
        assert_eq!(class.raw(), 1000);
        assert_eq!(
            class.to_string(),
            "libgit2 error class 1000 is not named by git2/errors.h"
        );
        let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(class);
        assert!(boxed.source().is_none());
        let sent = std::thread::spawn(move || boxed.to_string())
            .join()
            .unwrap();
        assert_eq!(sent, class.to_string());
        assert_eq!(code, UnnamedErrorCode(-2));
        assert_eq!(class.clone(), class);
    }

    #[test]
    fn from_raw_is_usable_in_const_context() {
        const NOT_FOUND: Option<ErrorCode> = ErrorCode::from_raw(-3);
        const OS: c_int = ErrorClass::Os.raw();
        assert_eq!(NOT_FOUND, Some(ErrorCode::NotFound));
        assert_eq!(OS, 2);
    }

    #[test]
    fn debug_names_the_variant() {
        assert_eq!(format!("{:?}", ErrorCode::NonFastForward), "NonFastForward");
        assert_eq!(format!("{:?}", ErrorClass::FetchHead), "FetchHead");
    }

    #[test]
    fn enums_are_int_sized() {
        assert_eq!(size_of::<ErrorCode>(), size_of::<c_int>());
        assert_eq!(size_of::<ErrorClass>(), size_of::<c_int>());
        assert_eq!(size_of::<Option<ErrorCode>>(), size_of::<c_int>());
    }

    mod sys_errors {
        use core::ffi::{c_int, c_void};
        use core::ptr::NonNull;
        use std::process::Command;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering::SeqCst;

        use glibc_bindings::libc::csu::errno_loc::{errno, set_errno};
        use glibc_bindings::libc::ffi as libc;

        use super::super::*;
        use crate::libgit2::src::util::alloc::{Alloc, Allocator};

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

        /// `git_error_last()`'s class and message, copied out.
        fn last_error() -> (c_int, Vec<u8>) {
            {
                // SAFETY: `git_error_last` never returns NULL, and its message lives until the
                // next libgit2 call on this thread; both are copied out before one.
                let e = unsafe { ffi::git_error_last() };
                // SAFETY: as above.
                let klass = unsafe { (*e).klass };
                // SAFETY: as above.
                let message2 = unsafe { (*e).message };
                // SAFETY: as above.
                unsafe { (klass, CStr::from_ptr(message2).to_bytes().to_vec()) }
            }
        }

        #[test]
        fn message_not_stored_reports_itself() {
            let e = MessageNotStored;
            assert_eq!(e.to_string(), "libgit2 could not store the error message");
            assert_eq!(format!("{e:?}"), "MessageNotStored");
            let boxed: Box<dyn std::error::Error> = Box::new(e);
            assert!(boxed.source().is_none());
        }

        /// The pin's own initialisation leaves the caller's `errno` alone, so an OS error
        /// set right after describes the caller's failure.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn first_pin_keeps_errno() {
            isolated(concat!(module_path!(), "::first_pin_keeps_errno"), || {
                set_errno(libc::ENOENT);
                assert!(runtime_pinned());
                assert_eq!(errno(), libc::ENOENT);
                set(ErrorClass::Os, c"open");
                let expected = b"open: No such file or directory".to_vec();
                assert_eq!(last_error(), (ErrorClass::Os.raw(), expected));
                assert_eq!(errno(), 0, "libgit2 consumed it");
            });
        }

        /// Later pins are a lookup, and keep `errno` too.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn later_pins_keep_errno() {
            assert!(runtime_pinned());
            set_errno(libc::EACCES);
            assert!(runtime_pinned());
            assert_eq!(errno(), libc::EACCES);
            set_errno(0);
        }

        /// Threads racing to the first pin each keep their own `errno`, and all see the
        /// one reference.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn racing_first_pins_keep_each_errno() {
            isolated(
                concat!(module_path!(), "::racing_first_pins_keep_each_errno"),
                || {
                    let barrier = std::sync::Barrier::new(8);
                    std::thread::scope(|s| {
                        for code in 1..=8 {
                            let barrier = &barrier;
                            s.spawn(move || {
                                barrier.wait();
                                set_errno(code);
                                assert!(runtime_pinned());
                                assert_eq!(errno(), code);
                            });
                        }
                    });
                    // SAFETY: no arguments; the extra reference dies with the process.
                    let count = unsafe { ffi::git_libgit2_init() };
                    assert_eq!(count, 2, "one pinned reference");
                },
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

        /// Installs [`Refusing`] as libgit2's allocator, before anything is allocated.
        fn install_refusing() {
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

        /// When libgit2 cannot be initialised, no wrapper reaches it: `clear`, `set`,
        /// `set_without_message` and `set_oom` do nothing, `set_str` fails, and the pin
        /// stays refused. (`errors.c` would write through a nonexistent TLS key.)
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn without_a_runtime_nothing_reaches_libgit2() {
            isolated(
                concat!(
                    module_path!(),
                    "::without_a_runtime_nothing_reaches_libgit2"
                ),
                || {
                    install_refusing();
                    REFUSING.store(true, SeqCst);
                    set_errno(libc::ENOENT);
                    clear();
                    assert_eq!(errno(), libc::ENOENT, "nothing consumed it");
                    set(ErrorClass::Os, c"open");
                    assert_eq!(errno(), libc::ENOENT);
                    set_without_message(ErrorClass::Os);
                    assert_eq!(errno(), libc::ENOENT);
                    assert_eq!(set_str(ErrorClass::Invalid, c"x"), Err(MessageNotStored));
                    set_oom();
                    assert!(!runtime_pinned(), "libgit2 could not initialise");
                    REFUSING.store(false, SeqCst);
                    assert!(!runtime_pinned(), "a refused pin is final");
                    assert_eq!(set_str(ErrorClass::Invalid, c"x"), Err(MessageNotStored));
                },
            );
        }
    }
}
