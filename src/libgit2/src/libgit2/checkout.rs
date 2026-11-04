//! Safe wrappers over `include/git2/checkout.h`.
//!
//! # Options
//!
//! A `git_checkout_options` tells a checkout what to write and how: the strategy and
//! notification flags, file and directory modes, the paths to restrict it to, the
//! baseline it compares the working directory against, where to write, the labels of
//! conflict markers, and three callbacks with their payloads. libgit2 never allocates
//! one it hands out, nor frees one it is given: the caller builds the struct (on the
//! stack, in C) with `git_checkout_options_init`, and every routine taking one reads it
//! through a `const git_checkout_options *`, copying it by value for the call
//! (`git_checkout_head`, `git_checkout_index`, `git_checkout_tree`, and through them
//! reset, stash, merge, cherry-pick, revert, rebase, clone, submodule update and
//! `git_worktree_add`, whose options embed one).
//!
//! So nothing it points at is owned: the strings, the path list, the baseline tree or
//! index and the payloads are the caller's, read during the calls the options are
//! passed to. Rust holds the struct by value as a [`CheckoutOptions<'o>`], which
//! [`CheckoutOptions::new`] fills through `git_checkout_options_init`, and borrows
//! everything it points at for `'o`: the getters live on [`CheckoutOptionsRef`], the
//! setters on [`CheckoutOptionsMut`], which is invariant in `'o`, so a setter only
//! accepts borrows that outlive the options.
//!
//! The baseline index is borrowed for `'o` as an [`IndexRef`], from the
//! [`IndexBox`](crate::libgit2::src::libgit2::index::IndexBox) or
//! [`IndexArc`](crate::libgit2::src::libgit2::index::IndexArc) holding it, and the baseline
//! tree as a [`TreeRef`], from the [`TreeArc`](crate::libgit2::src::libgit2::tree::TreeArc)
//! holding it. The pointers whose types this crate does not wrap yet stay raw seams,
//! with `unsafe` setters whose callers vouch for what the borrow checker cannot: the
//! notification, progress and performance callbacks with their payloads. The flags fields take the
//! raw bits of `git_checkout_strategy_t` and `git_checkout_notify_t` until those are wrapped.
//!
//! `git_rebase_init` and `git_rebase_open` are the one exception to "read during the
//! call": the rebase keeps a copy of the options, duplicating the strings and the
//! baseline tree but keeping the `paths` array, the callbacks and payloads, and the
//! baseline index as given, and `git_rebase_free` releases that index without having
//! taken a reference to it. Their wrappers must keep what the copy borrows alive for
//! the rebase, and must not hand them a baseline index.
//!
//! # Performance data
//!
//! A checkout that succeeds reports the filesystem calls it made to its performance-data
//! callback as a `git_checkout_perfdata`: three counters, zeroed when the checkout starts,
//! held in its own state and lent to the callback for the call. Rust reads them through a
//! [`CheckoutPerfdataRef`], and holds a copy, or a value of its own, as a
//! [`CheckoutPerfdata`].
//!
//! # Progress callbacks
//!
//! A [`ProgressCallback`] is a `git_checkout_progress_cb` over a Rust closure. A checkout
//! calls one synchronously, on its own thread, while it runs: once to report how many
//! steps it will take, then once per file it removes or writes and per conflict it
//! resolves, lending it the path for the call alone. The callback returns nothing, so it
//! observes the checkout without steering it. The wrappers of the routines that check out
//! store its trampoline and itself in the options they hand to C, for the call only, and
//! resume a panic its closure raised once C has returned; the options' own
//! [`set_progress_callback`](CheckoutOptionsMut::set_progress_callback) stays a raw seam
//! for C callbacks.

use core::any::Any;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self};
use std::panic::{self, AssertUnwindSafe};

use ffibox::{CBorrowedPtr, CCell, CMutSeam, CRefSeam, CZeroable, define_ctype};

use crate::ffi;
use crate::libgit2::src::libgit2::index::IndexRef;
use crate::libgit2::src::libgit2::strarray::StrArrayRef;
use crate::libgit2::src::libgit2::tree::TreeRef;

/// Options for a checkout (`git_checkout_options`), held by value in Rust storage.
///
/// Built by [`new`](Self::new) (or [`Default`]) with libgit2's defaults: a safe
/// checkout of every path into the repository's working directory, against `HEAD`,
/// without callbacks. Read it through [`as_ref`](Self::as_ref) and change it through
/// [`as_mut`](Self::as_mut).
///
/// `#[repr(transparent)]` over `git_checkout_options`, so a pointer to it is a pointer to
/// the C struct, which C reads in place. `'o` is the lifetime of what it points at: the
/// strings and path list its safe setters take are borrowed for `'o`, so the options
/// cannot outlive them:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;
/// use std::ffi::CString;
///
/// let mut options = CheckoutOptions::new();
/// {
///     let label = CString::new("mine").unwrap();
///     options.as_mut().set_our_label(Some(&label));
/// }
/// let _ = options.as_ref().our_label();
/// ```
///
/// The exclusive handle is invariant in `'o`, so one cannot be shrunk to accept a
/// shorter-lived borrow than its options were built for:
///
/// ```compile_fail
/// use libgit2::libgit2::src::libgit2::checkout::CheckoutOptionsMut;
///
/// fn shrink<'a, 'o>(m: CheckoutOptionsMut<'a, 'static>) -> CheckoutOptionsMut<'a, 'o> {
///     m
/// }
/// ```
///
/// Neither `Send` nor `Sync`: the raw seams store pointers to C objects and payloads
/// whose thread affinity the caller alone knows.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;
///
/// let options = CheckoutOptions::new();
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Not `Clone`: the payloads its callbacks receive may be exclusive borrows, which a
/// copy would alias. Build another with [`new`](Self::new) instead.
///
/// Wraps `ty::git_checkout_options`
#[doc(alias = "git_checkout_options")]
#[repr(transparent)]
pub struct CheckoutOptions<'o>(
    ffi::git_checkout_options,
    PhantomData<(*const (), &'o CStr)>,
);

/// Shared borrow of a [`CheckoutOptions`]. `Copy`, like `&T`; the getters live here.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CheckoutOptionsRef<'a, 'o>(CBorrowedPtr<'a, CheckoutOptions<'o>>);

/// Exclusive borrow of a [`CheckoutOptions`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
#[repr(transparent)]
pub struct CheckoutOptionsMut<'a, 'o>(
    CheckoutOptionsRef<'a, 'o>,
    // Invariant in the options, and so in `'o`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut CheckoutOptions<'o>>,
);

// SAFETY: `CheckoutOptions` is `#[repr(transparent)]` over `git_checkout_options` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, CheckoutOptions<'o>>` (`CheckoutOptionsMut` through
// `CheckoutOptionsRef`) with zero-sized markers and no `Drop`, hand out no reference to
// the options, and `CheckoutOptionsRef` has no write operation. `CheckoutOptionsMut` is
// invariant in the options through its `PhantomData<&'a mut CheckoutOptions<'o>>`.
unsafe impl<'o> CCell for CheckoutOptions<'o> {
    type C = ffi::git_checkout_options;
    type Mut<'a>
        = CheckoutOptionsMut<'a, 'o>
    where
        Self: 'a;
    type Ref<'a>
        = CheckoutOptionsRef<'a, 'o>
    where
        Self: 'a;
}

impl<'a, 'o: 'a> CRefSeam<'a> for CheckoutOptionsRef<'a, 'o> {
    type Cell = CheckoutOptions<'o>;
}

impl<'a, 'o: 'a> CMutSeam<'a> for CheckoutOptionsMut<'a, 'o> {
    type Cell = CheckoutOptions<'o>;
}

impl<'o> CheckoutOptions<'o> {
    /// libgit2's default options, as `GIT_CHECKOUT_OPTIONS_INIT` spells them: version 1,
    /// a safe checkout (strategy 0), every other field zero or NULL.
    ///
    /// `git_checkout_options_init` only fails for a version other than
    /// `GIT_CHECKOUT_OPTIONS_VERSION`, and with that version it reaches no global, no
    /// allocator and no thread state, so this needs no initialised library.
    ///
    /// Wraps `fn::git_checkout_options_init`
    ///
    /// # Panics
    ///
    /// Panics only if libgit2 refuses the version of the options it defines itself, which it never
    /// does.
    #[doc(alias = "git_checkout_options_init")]
    #[must_use]
    pub fn new() -> Self {
        let mut raw = MaybeUninit::<ffi::git_checkout_options>::uninit();
        // SAFETY: `raw` is writable storage for one `git_checkout_options`, which
        // `git_checkout_options_init` overwrites whole with the template for this
        // version; nothing else reads it during the call.
        let rc = unsafe {
            ffi::git_checkout_options_init(raw.as_mut_ptr(), ffi::GIT_CHECKOUT_OPTIONS_VERSION)
        };
        assert_eq!(rc, 0, "git_checkout_options_init refused its own version");
        // SAFETY: a 0 return means the template was copied in: every field is
        // initialised, every pointer NULL and every callback `None`.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    /// Shared handle to the options — the getters, and the pointer for C calls that
    /// read them.
    #[must_use]
    pub fn as_ref(&self) -> CheckoutOptionsRef<'_, 'o> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        CheckoutOptionsRef(unsafe { CBorrowedPtr::new(ptr::NonNull::from(self)) })
    }

    /// Exclusive handle to the options — the setters.
    pub fn as_mut(&mut self) -> CheckoutOptionsMut<'_, 'o> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(ptr::NonNull::from(self)) };
        CheckoutOptionsMut(CheckoutOptionsRef(ptr), PhantomData)
    }
}

impl Default for CheckoutOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CheckoutOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'o> CheckoutOptionsRef<'a, 'o> {
    /// The structure's version, `GIT_CHECKOUT_OPTIONS_VERSION`; fixed by
    /// [`CheckoutOptions::new`].
    ///
    /// Wraps `fld::git_checkout_options.version`
    #[doc(alias = "git_checkout_options.version")]
    #[must_use]
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let version_ptr = unsafe { &raw const (*p).version };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { version_ptr.read() }
    }

    /// The checkout strategy, the raw bits of `git_checkout_strategy_t`: 0, the default,
    /// is a safe checkout.
    ///
    /// Wraps `fld::git_checkout_options.checkout_strategy`
    #[doc(alias = "git_checkout_options.checkout_strategy")]
    #[must_use]
    pub fn strategy(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let checkout_strategy_ptr = unsafe { &raw const (*p).checkout_strategy };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { checkout_strategy_ptr.read() }
    }

    /// Whether filters such as CRLF conversion are skipped when files are written; no
    /// by default.
    ///
    /// Wraps `fld::git_checkout_options.disable_filters`
    #[doc(alias = "git_checkout_options.disable_filters")]
    #[must_use]
    pub fn disable_filters(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let disable_filters_ptr = unsafe { &raw const (*p).disable_filters };
        // SAFETY: a read through the shared handle's pointer to live options.
        let disable = unsafe { disable_filters_ptr.read() };
        disable != 0
    }

    /// The mode directories are created with; 0, the default, means `0755`.
    ///
    /// Wraps `fld::git_checkout_options.dir_mode`
    #[doc(alias = "git_checkout_options.dir_mode")]
    #[must_use]
    pub fn dir_mode(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let dir_mode_ptr = unsafe { &raw const (*p).dir_mode };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { dir_mode_ptr.read() }
    }

    /// The mode files are created with; 0, the default, means the blob's own (`0644`
    /// or `0755`).
    ///
    /// Wraps `fld::git_checkout_options.file_mode`
    #[doc(alias = "git_checkout_options.file_mode")]
    #[must_use]
    pub fn file_mode(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let file_mode_ptr = unsafe { &raw const (*p).file_mode };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { file_mode_ptr.read() }
    }

    /// The `open(2)` flags files are written with; 0, the default, and any negative
    /// value mean `O_CREAT | O_TRUNC | O_WRONLY`.
    ///
    /// Wraps `fld::git_checkout_options.file_open_flags`
    #[doc(alias = "git_checkout_options.file_open_flags")]
    #[must_use]
    pub fn file_open_flags(&self) -> i32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let file_open_flags_ptr = unsafe { &raw const (*p).file_open_flags };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { file_open_flags_ptr.read() }
    }

    /// The situations the notification callback is called for, the raw bits of
    /// `git_checkout_notify_t`; none by default.
    ///
    /// Wraps `fld::git_checkout_options.notify_flags`
    #[doc(alias = "git_checkout_options.notify_flags")]
    #[must_use]
    pub fn notify_flags(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let notify_flags_ptr = unsafe { &raw const (*p).notify_flags };
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { notify_flags_ptr.read() }
    }

    /// The notification callback and its payload; `None` and NULL by default.
    ///
    /// Raw seam: the callback is `git_checkout_notify_cb`, which this crate does not
    /// wrap yet.
    ///
    /// Wraps `fld::git_checkout_options.notify_cb`
    /// Wraps `fld::git_checkout_options.notify_payload`
    #[doc(alias = "git_checkout_options.notify_cb")]
    #[doc(alias = "git_checkout_options.notify_payload")]
    #[must_use]
    pub fn notify_callback(&self) -> (ffi::git_checkout_notify_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let notify_cb_ptr = unsafe { &raw const (*p).notify_cb };
        // SAFETY: as above.
        let notify_payload_ptr = unsafe { &raw const (*p).notify_payload };
        // SAFETY: reads the initialised field.
        let notify_cb = unsafe { notify_cb_ptr.read() };
        // SAFETY: as above.
        let notify_payload = unsafe { notify_payload_ptr.read() };
        (notify_cb, notify_payload)
    }

    /// The progress callback and its payload; `None` and NULL by default.
    ///
    /// Raw seam: the fields hold whatever C callback and payload were stored. The safe
    /// form of the callback is a [`ProgressCallback`], which the wrappers of the checkout
    /// routines hand to libgit2 for the call.
    ///
    /// Wraps `fld::git_checkout_options.progress_cb`
    /// Wraps `fld::git_checkout_options.progress_payload`
    #[doc(alias = "git_checkout_options.progress_cb")]
    #[doc(alias = "git_checkout_options.progress_payload")]
    #[must_use]
    pub fn progress_callback(&self) -> (ffi::git_checkout_progress_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let progress_cb_ptr = unsafe { &raw const (*p).progress_cb };
        // SAFETY: as above.
        let progress_payload_ptr = unsafe { &raw const (*p).progress_payload };
        // SAFETY: reads the initialised field.
        let progress_cb = unsafe { progress_cb_ptr.read() };
        // SAFETY: as above.
        let progress_payload = unsafe { progress_payload_ptr.read() };
        (progress_cb, progress_payload)
    }

    /// The performance-data callback and its payload; `None` and NULL by default.
    ///
    /// Raw seam: the callback is `git_checkout_perfdata_cb`, which this crate does not
    /// wrap yet.
    ///
    /// Wraps `fld::git_checkout_options.perfdata_cb`
    /// Wraps `fld::git_checkout_options.perfdata_payload`
    #[doc(alias = "git_checkout_options.perfdata_cb")]
    #[doc(alias = "git_checkout_options.perfdata_payload")]
    #[must_use]
    pub fn perfdata_callback(&self) -> (ffi::git_checkout_perfdata_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let perfdata_cb_ptr = unsafe { &raw const (*p).perfdata_cb };
        // SAFETY: as above.
        let perfdata_payload_ptr = unsafe { &raw const (*p).perfdata_payload };
        // SAFETY: reads the initialised field.
        let perfdata_cb = unsafe { perfdata_cb_ptr.read() };
        // SAFETY: as above.
        let perfdata_payload = unsafe { perfdata_payload_ptr.read() };
        (perfdata_cb, perfdata_payload)
    }

    /// The paths, or wildmatch patterns, the checkout is restricted to; empty, the
    /// default, means every path. Borrowed from the options, since the array header is
    /// stored in them.
    ///
    /// Wraps `fld::git_checkout_options.paths`
    #[doc(alias = "git_checkout_options.paths")]
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a field address is not NULL"
    )]
    pub fn paths(&self) -> StrArrayRef<'a> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses the live options, which outlive `'a`; the
        // projection forms no reference to them.
        let paths_ptr = unsafe { &raw const (*p).paths };
        // SAFETY: `paths_ptr` addresses the options' embedded `git_strarray`. Every setter leaves
        // it NULL with count 0, or a copy of an array a `StrArrayRef<'o>` vouched for, whose
        // strings outlive `'o` and so `'a`; nothing writes it while this shared handle lives, and
        // the result has no write operation.
        let paths = unsafe { StrArrayRef::from_ptr(paths_ptr.cast_mut()) };
        paths.expect("a field address is not NULL")
    }

    /// The baseline tree the working directory is compared against; `None`, the
    /// default, means `HEAD`'s (unless a [`baseline_index`](Self::baseline_index) is
    /// set). Borrowed for `'o`, as it was set.
    ///
    /// Wraps `fld::git_checkout_options.baseline`
    #[doc(alias = "git_checkout_options.baseline")]
    #[must_use]
    pub fn baseline(&self) -> Option<TreeRef<'o, 'o>> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let baseline_ptr = unsafe { &raw const (*p).baseline };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let baseline = unsafe { baseline_ptr.read() };
        // SAFETY: every setter stores NULL or a `TreeRef<'o, 'o>`'s pointer, a tree its holder
        // keeps live and nobody writes for `'o`, of a repository outliving `'o`, which this handle
        // only lends on.
        unsafe { TreeRef::from_ptr(baseline) }
    }

    /// The baseline index, which overrides [`baseline`](Self::baseline); `None` by
    /// default. Borrowed for `'o`, as it was set.
    ///
    /// Wraps `fld::git_checkout_options.baseline_index`
    #[doc(alias = "git_checkout_options.baseline_index")]
    #[must_use]
    pub fn baseline_index(&self) -> Option<IndexRef<'o>> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let index_ptr = unsafe { &raw const (*p).baseline_index };
        // SAFETY: reads the initialised field; nothing writes it while the options are shared.
        let index = unsafe { index_ptr.read() };
        // SAFETY: every setter stores NULL or an `IndexRef<'o>`'s pointer, an index its holder
        // keeps live for `'o`, which this handle only lends on.
        unsafe { IndexRef::from_ptr(index) }
    }

    /// The directory files are written to; `None`, the default, means the repository's
    /// working directory.
    ///
    /// Wraps `fld::git_checkout_options.target_directory`
    #[doc(alias = "git_checkout_options.target_directory")]
    #[must_use]
    pub fn target_directory(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let target_directory_ptr = unsafe { &raw const (*p).target_directory };
        // SAFETY: reads the initialised field; nothing writes it while the handle is shared.
        let s = unsafe { target_directory_ptr.read() };
        // SAFETY: every setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
    }

    /// The name of the common ancestor's side in conflict markers; `None`, the
    /// default, means `"ancestor"`.
    ///
    /// Wraps `fld::git_checkout_options.ancestor_label`
    #[doc(alias = "git_checkout_options.ancestor_label")]
    #[must_use]
    pub fn ancestor_label(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let ancestor_label_ptr = unsafe { &raw const (*p).ancestor_label };
        // SAFETY: reads the initialised field; nothing writes it while the handle is shared.
        let s = unsafe { ancestor_label_ptr.read() };
        // SAFETY: every setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
    }

    /// The name of "our" side in conflict markers; `None`, the default, means
    /// `"ours"`.
    ///
    /// Wraps `fld::git_checkout_options.our_label`
    #[doc(alias = "git_checkout_options.our_label")]
    #[must_use]
    pub fn our_label(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let our_label_ptr = unsafe { &raw const (*p).our_label };
        // SAFETY: reads the initialised field; nothing writes it while the handle is shared.
        let s = unsafe { our_label_ptr.read() };
        // SAFETY: every setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
    }

    /// The name of "their" side in conflict markers; `None`, the default, means
    /// `"theirs"`.
    ///
    /// Wraps `fld::git_checkout_options.their_label`
    #[doc(alias = "git_checkout_options.their_label")]
    #[must_use]
    pub fn their_label(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: the shared handle's pointer addresses live options; the projection forms no
        // reference to them.
        let their_label_ptr = unsafe { &raw const (*p).their_label };
        // SAFETY: reads the initialised field; nothing writes it while the handle is shared.
        let s = unsafe { their_label_ptr.read() };
        // SAFETY: every setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
    }
}

impl fmt::Debug for CheckoutOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (notify, _) = self.notify_callback();
        let (progress, _) = self.progress_callback();
        let (perfdata, _) = self.perfdata_callback();
        f.debug_struct("CheckoutOptions")
            .field("version", &self.version())
            .field("strategy", &format_args!("{:#x}", self.strategy()))
            .field("disable_filters", &self.disable_filters())
            .field("dir_mode", &format_args!("{:#o}", self.dir_mode()))
            .field("file_mode", &format_args!("{:#o}", self.file_mode()))
            .field(
                "file_open_flags",
                &format_args!("{:#x}", self.file_open_flags()),
            )
            .field("notify_flags", &format_args!("{:#x}", self.notify_flags()))
            .field("notify_callback", &notify.is_some())
            .field("progress_callback", &progress.is_some())
            .field("paths", &self.paths())
            .field("baseline", &self.baseline().is_some())
            .field("baseline_index", &self.baseline_index().is_some())
            .field("target_directory", &self.target_directory())
            .field("ancestor_label", &self.ancestor_label())
            .field("our_label", &self.our_label())
            .field("their_label", &self.their_label())
            .field("perfdata_callback", &perfdata.is_some())
            .finish()
    }
}

impl<'o> CheckoutOptionsMut<'_, 'o> {
    /// Reborrows shared.
    #[must_use]
    pub fn as_ref(&self) -> CheckoutOptionsRef<'_, 'o> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> CheckoutOptionsMut<'_, 'o> {
        CheckoutOptionsMut(self.0, PhantomData)
    }

    /// Sets the checkout strategy, the raw bits of `git_checkout_strategy_t`. libgit2
    /// tests the bits it knows and ignores the rest.
    ///
    /// Wraps `fld::git_checkout_options.checkout_strategy`
    #[doc(alias = "git_checkout_options.checkout_strategy")]
    pub fn set_strategy(&mut self, strategy: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let checkout_strategy_ptr = unsafe { &raw mut (*p).checkout_strategy };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { checkout_strategy_ptr.write(strategy) }
    }

    /// Sets whether filters such as CRLF conversion are skipped when files are
    /// written.
    ///
    /// Wraps `fld::git_checkout_options.disable_filters`
    #[doc(alias = "git_checkout_options.disable_filters")]
    pub fn set_disable_filters(&mut self, disable: bool) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let disable_filters_ptr = unsafe { &raw mut (*p).disable_filters };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { disable_filters_ptr.write(c_int::from(disable)) }
    }

    /// Sets the mode directories are created with; 0 means `0755`.
    ///
    /// Wraps `fld::git_checkout_options.dir_mode`
    #[doc(alias = "git_checkout_options.dir_mode")]
    pub fn set_dir_mode(&mut self, mode: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let dir_mode_ptr = unsafe { &raw mut (*p).dir_mode };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { dir_mode_ptr.write(mode) }
    }

    /// Sets the mode files are created with; 0 means the blob's own.
    ///
    /// Wraps `fld::git_checkout_options.file_mode`
    #[doc(alias = "git_checkout_options.file_mode")]
    pub fn set_file_mode(&mut self, mode: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let file_mode_ptr = unsafe { &raw mut (*p).file_mode };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { file_mode_ptr.write(mode) }
    }

    /// Sets the `open(2)` flags files are written with; 0 or a negative value means
    /// `O_CREAT | O_TRUNC | O_WRONLY`. Flags that cannot write the file (`O_EXCL` over
    /// an existing one, say) make the checkout fail with libgit2's error.
    ///
    /// Wraps `fld::git_checkout_options.file_open_flags`
    #[doc(alias = "git_checkout_options.file_open_flags")]
    pub fn set_file_open_flags(&mut self, flags: i32) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let file_open_flags_ptr = unsafe { &raw mut (*p).file_open_flags };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { file_open_flags_ptr.write(flags) }
    }

    /// Sets the situations the notification callback is called for, the raw bits of
    /// `git_checkout_notify_t`.
    ///
    /// Wraps `fld::git_checkout_options.notify_flags`
    #[doc(alias = "git_checkout_options.notify_flags")]
    pub fn set_notify_flags(&mut self, flags: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let notify_flags_ptr = unsafe { &raw mut (*p).notify_flags };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { notify_flags_ptr.write(flags) }
    }

    /// Sets the notification callback and the payload it is passed; `None` calls
    /// nothing. libgit2 calls it, for the situations in
    /// [`notify_flags`](CheckoutOptionsRef::notify_flags), before it changes any file;
    /// a non-zero return cancels the checkout, which returns that value.
    ///
    /// Raw seam: the callback is `git_checkout_notify_cb`, which this crate does not
    /// wrap yet.
    ///
    /// # Safety
    ///
    /// Until these options are dropped or the callback is replaced, every libgit2 call
    /// they are passed to may call `callback` on the calling thread, any number of
    /// times, with arguments valid for that call only and with `payload`: `callback`
    /// must be sound to call that way, and `payload` must stay valid for whatever
    /// `callback` does with it.
    ///
    /// Wraps `fld::git_checkout_options.notify_cb`
    /// Wraps `fld::git_checkout_options.notify_payload`
    #[doc(alias = "git_checkout_options.notify_cb")]
    #[doc(alias = "git_checkout_options.notify_payload")]
    pub unsafe fn set_notify_callback(
        &mut self,
        callback: ffi::git_checkout_notify_cb,
        payload: *mut c_void,
    ) {
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let notify_cb_ptr = unsafe { &raw mut (*p).notify_cb };
        // SAFETY: as above.
        let notify_payload_ptr = unsafe { &raw mut (*p).notify_payload };
        // SAFETY: the fields are plain data, so the old values need no drop.
        unsafe { notify_cb_ptr.write(callback) };
        // SAFETY: as above.
        unsafe { notify_payload_ptr.write(payload) };
    }

    /// Sets the progress callback and the payload it is passed; `None` calls nothing.
    /// libgit2 reports each file it writes or removes, and the total.
    ///
    /// Raw seam: stores a C callback and payload as given. The safe form of the callback
    /// is a [`ProgressCallback`], which the wrappers of the checkout routines hand to
    /// libgit2 for the call, with its panics contained.
    ///
    /// # Safety
    ///
    /// As [`set_notify_callback`](Self::set_notify_callback), for
    /// `git_checkout_progress_cb`'s arguments: the path may be NULL.
    ///
    /// Wraps `fld::git_checkout_options.progress_cb`
    /// Wraps `fld::git_checkout_options.progress_payload`
    #[doc(alias = "git_checkout_options.progress_cb")]
    #[doc(alias = "git_checkout_options.progress_payload")]
    pub unsafe fn set_progress_callback(
        &mut self,
        callback: ffi::git_checkout_progress_cb,
        payload: *mut c_void,
    ) {
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let progress_cb_ptr = unsafe { &raw mut (*p).progress_cb };
        // SAFETY: as above.
        let progress_payload_ptr = unsafe { &raw mut (*p).progress_payload };
        // SAFETY: the fields are plain data, so the old values need no drop.
        unsafe { progress_cb_ptr.write(callback) };
        // SAFETY: as above.
        unsafe { progress_payload_ptr.write(payload) };
    }

    /// Sets the performance-data callback and the payload it is passed; `None` calls
    /// nothing. libgit2 calls it once a checkout succeeds, with counts of the
    /// `mkdir`, `stat` and `chmod` calls it made, which the callback reads by borrowing a
    /// [`CheckoutPerfdataRef`] over its `perfdata` argument for the call.
    ///
    /// Raw seam: the callback is `git_checkout_perfdata_cb`, which this crate does not
    /// wrap yet.
    ///
    /// # Safety
    ///
    /// As [`set_notify_callback`](Self::set_notify_callback), for
    /// `git_checkout_perfdata_cb`'s arguments.
    ///
    /// Wraps `fld::git_checkout_options.perfdata_cb`
    /// Wraps `fld::git_checkout_options.perfdata_payload`
    #[doc(alias = "git_checkout_options.perfdata_cb")]
    #[doc(alias = "git_checkout_options.perfdata_payload")]
    pub unsafe fn set_perfdata_callback(
        &mut self,
        callback: ffi::git_checkout_perfdata_cb,
        payload: *mut c_void,
    ) {
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let perfdata_cb_ptr = unsafe { &raw mut (*p).perfdata_cb };
        // SAFETY: as above.
        let perfdata_payload_ptr = unsafe { &raw mut (*p).perfdata_payload };
        // SAFETY: the fields are plain data, so the old values need no drop.
        unsafe { perfdata_cb_ptr.write(callback) };
        // SAFETY: as above.
        unsafe { perfdata_payload_ptr.write(payload) };
    }

    /// Restricts the checkout to `paths`: wildmatch patterns, or exact paths with the
    /// `GIT_CHECKOUT_DISABLE_PATHSPEC_MATCH` strategy bit. libgit2 reads the array and
    /// its strings during each call, and copies what it keeps; an empty array means
    /// every path.
    ///
    /// The array is borrowed for `'o`, from a [`StrArrayView`] over borrowed strings or
    /// an owned [`StrArrayVal`](crate::libgit2::src::libgit2::strarray::StrArrayVal), which can
    /// then not be changed while the options live.
    ///
    /// [`StrArrayView`]: crate::libgit2::src::libgit2::strarray::StrArrayView
    ///
    /// Wraps `fld::git_checkout_options.paths`
    #[doc(alias = "git_checkout_options.paths")]
    pub fn set_paths(&mut self, paths: StrArrayRef<'o>) {
        // A NULL array reads as empty whatever its count; C would read `count` strings.
        let count = paths.len();
        // SAFETY: the shared handle's pointer addresses a live `git_strarray`; the projection forms
        // no reference to it.
        let strings_ptr = unsafe { &raw const (*paths.as_ptr()).strings };
        // SAFETY: reads the initialised field.
        let strings = unsafe { strings_ptr.read() };
        let raw = ffi::git_strarray {
            strings: if count == 0 { ptr::null_mut() } else { strings },
            count,
        };
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let paths_ptr2 = unsafe { &raw mut (*p).paths };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data. `raw` is NULL with count 0, or `count` non-NULL strings in
        // an array that `paths` vouches for for `'o`, as `paths()` requires.
        unsafe { paths_ptr2.write(raw) }
    }

    /// Lifts the path restriction: every path is checked out.
    ///
    /// Wraps `fld::git_checkout_options.paths`
    #[doc(alias = "git_checkout_options.paths")]
    pub fn clear_paths(&mut self) {
        let empty = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 0,
        };
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let paths_ptr = unsafe { &raw mut (*p).paths };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { paths_ptr.write(empty) }
    }

    /// Sets the baseline tree the working directory is compared against; `None` means
    /// `HEAD`'s. libgit2 only reads it, taking a reference of its own for each call
    /// (and a rebase, for as long as it lives).
    ///
    /// The tree is borrowed for `'o` from the owner holding it, which then outlives the
    /// options, as does the repository the tree was looked up in.
    ///
    /// Wraps `fld::git_checkout_options.baseline`
    #[doc(alias = "git_checkout_options.baseline")]
    pub fn set_baseline(&mut self, tree: Option<TreeRef<'o, 'o>>) {
        let tree = tree.map_or(ptr::null_mut(), |t| t.as_ptr().cast_mut());
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let baseline_ptr = unsafe { &raw mut (*p).baseline };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop. The tree is NULL or one a
        // `TreeRef<'o, 'o>` vouches for, as `baseline()` requires.
        unsafe { baseline_ptr.write(tree) }
    }

    /// Sets the baseline index, which overrides the [baseline
    /// tree](Self::set_baseline); `None` means none. libgit2 snapshots it for each call,
    /// counting a reference of its own for the call only.
    ///
    /// The index is borrowed for `'o` from the owner holding it, which then outlives the
    /// options, and an [`IndexBox`](crate::libgit2::src::libgit2::index::IndexBox) lends no
    /// exclusive handle meanwhile.
    ///
    /// Wraps `fld::git_checkout_options.baseline_index`
    #[doc(alias = "git_checkout_options.baseline_index")]
    pub fn set_baseline_index(&mut self, index: Option<IndexRef<'o>>) {
        let index = index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut());
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let baseline_index_ptr = unsafe { &raw mut (*p).baseline_index };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop. The index is NULL or one
        // an `IndexRef<'o>` vouches for, as `baseline_index()` requires.
        unsafe { baseline_index_ptr.write(index) }
    }

    /// Sets the directory files are written to, created when missing; `None` means
    /// the repository's working directory, which a bare repository lacks.
    ///
    /// Wraps `fld::git_checkout_options.target_directory`
    #[doc(alias = "git_checkout_options.target_directory")]
    pub fn set_target_directory(&mut self, directory: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let directory_ptr = unsafe { &raw mut (*p).target_directory };
        // SAFETY: the field is plain data, so the old value needs no drop.
        unsafe { directory_ptr.write(directory.map_or(ptr::null(), CStr::as_ptr)) };
    }

    /// Sets the name of the common ancestor's side in conflict markers; `None` means
    /// `"ancestor"`.
    ///
    /// Wraps `fld::git_checkout_options.ancestor_label`
    #[doc(alias = "git_checkout_options.ancestor_label")]
    pub fn set_ancestor_label(&mut self, label: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let ancestor_label_ptr = unsafe { &raw mut (*p).ancestor_label };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { ancestor_label_ptr.write(label.map_or(ptr::null(), CStr::as_ptr)) }
    }

    /// Sets the name of "our" side in conflict markers; `None` means `"ours"`.
    ///
    /// Wraps `fld::git_checkout_options.our_label`
    #[doc(alias = "git_checkout_options.our_label")]
    pub fn set_our_label(&mut self, label: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let our_label_ptr = unsafe { &raw mut (*p).our_label };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { our_label_ptr.write(label.map_or(ptr::null(), CStr::as_ptr)) }
    }

    /// Sets the name of "their" side in conflict markers; `None` means `"theirs"`.
    ///
    /// Wraps `fld::git_checkout_options.their_label`
    #[doc(alias = "git_checkout_options.their_label")]
    pub fn set_their_label(&mut self, label: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: `p` is the handle's pointer to the live object it borrows; the projection forms
        // no reference to it.
        let their_label_ptr = unsafe { &raw mut (*p).their_label };
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { their_label_ptr.write(label.map_or(ptr::null(), CStr::as_ptr)) }
    }
}

impl fmt::Debug for CheckoutOptionsMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `CheckoutOptions` is a `git_checkout_options`, so C reads a value Rust holds in place.
const _: () = assert!(
    size_of::<CheckoutOptions<'_>>() == size_of::<ffi::git_checkout_options>()
        && align_of::<CheckoutOptions<'_>>() == align_of::<ffi::git_checkout_options>()
);

define_ctype!(
    /// The counts of filesystem calls a checkout made, as libgit2 reports them to the
    /// [performance-data callback](CheckoutOptionsMut::set_perfdata_callback): its `mkdir`,
    /// `stat` and `chmod` calls.
    ///
    /// Plain data: three counters, no pointer and nothing to release. libgit2 counts into
    /// one zeroed at the start of each checkout, inside the checkout's own state, and lends
    /// it to the callback for the call only, as a `const git_checkout_perfdata *`: a
    /// [`CheckoutPerfdataRef`] borrowed for the callback reads it there
    /// ([`CRefSeam::from_ptr`]), and [`copied`](CheckoutPerfdataRef::copied) keeps the
    /// counts beyond it. A value Rust holds is built with [`zeroed`](CZeroable::zeroed) (or
    /// [`Default`]) and is `Copy`, like the C struct.
    ///
    /// Wraps `ty::git_checkout_perfdata`
    #[doc(alias = "git_checkout_perfdata")]
    #[derive(Clone, Copy)]
    CheckoutPerfdata,
    CheckoutPerfdataRef,
    CheckoutPerfdataMut,
    ffi::git_checkout_perfdata
);

// SAFETY: three `size_t` counters, all zero being "no call counted yet": the state libgit2
// starts every checkout's counts from (`checkout_data data = {0}`), and a value every
// accessor reads. Nothing frees it.
unsafe impl CZeroable for CheckoutPerfdata {}

// SAFETY: a `CheckoutPerfdata` is three plain `usize`s: no pointer, no interior mutability,
// no thread affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for CheckoutPerfdata {}

// SAFETY: `CheckoutPerfdataMut` is an exclusive borrow of such a value, as
// `&mut CheckoutPerfdata`.
unsafe impl Send for CheckoutPerfdataMut<'_> {}

// SAFETY: shared access only reads the counters (every write goes through
// `CheckoutPerfdataMut`, which excludes other handles, and libgit2 writes none while it
// lends them to its callback), so concurrent readers race nothing.
unsafe impl Sync for CheckoutPerfdata {}

// SAFETY: as above; `CheckoutPerfdataRef` is a shared borrow, as `&CheckoutPerfdata`.
unsafe impl Send for CheckoutPerfdataRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for CheckoutPerfdataRef<'_> {}

// SAFETY: `&CheckoutPerfdataMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for CheckoutPerfdataMut<'_> {}

impl Default for CheckoutPerfdata {
    /// No call counted: what a checkout starts from.
    fn default() -> Self {
        Self::zeroed()
    }
}

impl CheckoutPerfdataRef<'_> {
    /// How many directories the checkout created (`mkdir` calls).
    ///
    /// Wraps `fld::git_checkout_perfdata.mkdir_calls`
    #[doc(alias = "git_checkout_perfdata.mkdir_calls")]
    #[must_use]
    pub fn mkdir_calls(&self) -> usize {
        // SAFETY: `self.as_ptr()` addresses the live counts this handle borrows; the
        // projection forms no reference to them.
        let mkdir_calls_ptr = unsafe { &raw const (*self.as_ptr()).mkdir_calls };
        // SAFETY: an initialised `size_t`, unwritten while the handle borrows it.
        unsafe { mkdir_calls_ptr.read() }
    }

    /// How many times the checkout examined a path on disk (`stat` and `lstat` calls).
    ///
    /// Wraps `fld::git_checkout_perfdata.stat_calls`
    #[doc(alias = "git_checkout_perfdata.stat_calls")]
    #[must_use]
    pub fn stat_calls(&self) -> usize {
        // SAFETY: `self.as_ptr()` addresses the live counts this handle borrows; the
        // projection forms no reference to them.
        let stat_calls_ptr = unsafe { &raw const (*self.as_ptr()).stat_calls };
        // SAFETY: an initialised `size_t`, unwritten while the handle borrows it.
        unsafe { stat_calls_ptr.read() }
    }

    /// How many times the checkout changed a directory's mode (`chmod` calls).
    ///
    /// Wraps `fld::git_checkout_perfdata.chmod_calls`
    #[doc(alias = "git_checkout_perfdata.chmod_calls")]
    #[must_use]
    pub fn chmod_calls(&self) -> usize {
        // SAFETY: `self.as_ptr()` addresses the live counts this handle borrows; the
        // projection forms no reference to them.
        let chmod_calls_ptr = unsafe { &raw const (*self.as_ptr()).chmod_calls };
        // SAFETY: an initialised `size_t`, unwritten while the handle borrows it.
        unsafe { chmod_calls_ptr.read() }
    }

    /// A copy of the counts, which outlives the borrow: from a callback, the counts libgit2
    /// lent it for the call.
    #[must_use]
    pub fn copied(&self) -> CheckoutPerfdata {
        let mut copy = CheckoutPerfdata::zeroed();
        let mut m = copy.as_mut();
        m.set_mkdir_calls(self.mkdir_calls());
        m.set_stat_calls(self.stat_calls());
        m.set_chmod_calls(self.chmod_calls());
        copy
    }
}

impl CheckoutPerfdataMut<'_> {
    /// Sets the count of directories created.
    ///
    /// Wraps `fld::git_checkout_perfdata.mkdir_calls`
    #[doc(alias = "git_checkout_perfdata.mkdir_calls")]
    pub fn set_mkdir_calls(&mut self, calls: usize) {
        // SAFETY: `self.as_mut_ptr()` addresses the live counts this handle borrows
        // exclusively; the projection forms no reference to them.
        let mkdir_calls_ptr = unsafe { &raw mut (*self.as_mut_ptr()).mkdir_calls };
        // SAFETY: a write of plain data through the exclusive handle's pointer.
        unsafe { mkdir_calls_ptr.write(calls) }
    }

    /// Sets the count of paths examined on disk.
    ///
    /// Wraps `fld::git_checkout_perfdata.stat_calls`
    #[doc(alias = "git_checkout_perfdata.stat_calls")]
    pub fn set_stat_calls(&mut self, calls: usize) {
        // SAFETY: `self.as_mut_ptr()` addresses the live counts this handle borrows
        // exclusively; the projection forms no reference to them.
        let stat_calls_ptr = unsafe { &raw mut (*self.as_mut_ptr()).stat_calls };
        // SAFETY: a write of plain data through the exclusive handle's pointer.
        unsafe { stat_calls_ptr.write(calls) }
    }

    /// Sets the count of directory mode changes.
    ///
    /// Wraps `fld::git_checkout_perfdata.chmod_calls`
    #[doc(alias = "git_checkout_perfdata.chmod_calls")]
    pub fn set_chmod_calls(&mut self, calls: usize) {
        // SAFETY: `self.as_mut_ptr()` addresses the live counts this handle borrows
        // exclusively; the projection forms no reference to them.
        let chmod_calls_ptr = unsafe { &raw mut (*self.as_mut_ptr()).chmod_calls };
        // SAFETY: a write of plain data through the exclusive handle's pointer.
        unsafe { chmod_calls_ptr.write(calls) }
    }
}

impl fmt::Debug for CheckoutPerfdataRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckoutPerfdata")
            .field("mkdir_calls", &self.mkdir_calls())
            .field("stat_calls", &self.stat_calls())
            .field("chmod_calls", &self.chmod_calls())
            .finish()
    }
}

impl fmt::Debug for CheckoutPerfdataMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for CheckoutPerfdata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl PartialEq for CheckoutPerfdata {
    /// Counter by counter.
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_ref(), other.as_ref());
        a.mkdir_calls() == b.mkdir_calls()
            && a.stat_calls() == b.stat_calls()
            && a.chmod_calls() == b.chmod_calls()
    }
}

impl Eq for CheckoutPerfdata {}

// A `CheckoutPerfdata` is a `git_checkout_perfdata`, so a callback's pointer is one to it.
const _: () = assert!(
    size_of::<CheckoutPerfdata>() == size_of::<ffi::git_checkout_perfdata>()
        && align_of::<CheckoutPerfdata>() == align_of::<ffi::git_checkout_perfdata>()
);

/// The closure behind a [`ProgressCallback`].
type ProgressFn<'f> = dyn FnMut(Option<&CStr>, usize, usize) + 'f;

/// A `git_checkout_progress_cb`: the callback a checkout calls as it goes, with the path
/// it has just written or removed, the steps it has completed and the steps it will take.
///
/// Built over a closure with [`new`](Self::new). libgit2 calls it synchronously, on the
/// thread running the checkout: once before the first step, with no path and no completed
/// step, to report the total, then once after each file it removes, writes or resolves a
/// conflict of, with that path, relative to the working directory, and the count so far.
/// The path is lent for the call only (it is the diff's or the checkout's own string), so
/// the closure receives it as a `&CStr` that cannot outlive the call. A dry run, or a
/// checkout that stops before it writes anything, never calls it.
///
/// The callback returns nothing, so it cannot cancel the checkout. A panic in the closure
/// does not cross into C: the checkout goes on, every later call it makes returns without
/// running the closure, and the panic resumes once control is back in Rust, from
/// [`call`](Self::call) or from the wrapper that ran the checkout.
///
/// ```
/// use std::ffi::CStr;
///
/// use libgit2::libgit2::src::libgit2::checkout::ProgressCallback;
///
/// let mut seen = Vec::new();
/// let mut record = |path: Option<&CStr>, completed: usize, total: usize| {
///     seen.push((path.map(CStr::to_owned), completed, total));
/// };
/// let mut callback = ProgressCallback::new(&mut record);
/// callback.call(None, 0, 2);
/// callback.call(Some(c"a.txt"), 1, 2);
/// callback.call(Some(c"dir/b.txt"), 2, 2);
/// drop(callback);
/// assert_eq!(seen[0], (None, 0, 2));
/// assert_eq!(seen[2], (Some(c"dir/b.txt".to_owned()), 2, 2));
/// ```
///
/// Wraps `ty::git_checkout_progress_cb`
#[doc(alias = "git_checkout_progress_cb")]
pub struct ProgressCallback<'f> {
    f: &'f mut ProgressFn<'f>,
    /// A panic the closure raised inside C, resumed once control is back in Rust.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> ProgressCallback<'f> {
    /// The callback that calls `f` with each path and step count.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(Option<&CStr>, usize, usize) + 'f,
    {
        Self { f, panic: None }
    }

    /// Calls the callback with `path`, `completed` and `total`, through the trampoline
    /// libgit2 calls. A panic of the closure resumes here.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic but by resuming the closure's panic: the trampoline is never NULL"
    )]
    pub fn call(&mut self, path: Option<&CStr>, completed: usize, total: usize) {
        self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            let path = path.map_or(ptr::null(), CStr::as_ptr);
            // SAFETY: `payload` is this callback, which nothing else uses during the call;
            // `path` NULL or a NUL-terminated string borrowed for the call: the
            // trampoline's contract.
            unsafe { callback(path, completed, total, payload) }
        });
    }

    /// Runs `walk` with the C callback and its payload, as the wrapper of a routine that
    /// checks out (`git_checkout_head`, `git_checkout_index`, `git_checkout_tree`, and the
    /// routines that check out through them) stores them in the options it hands to C,
    /// then resumes a panic the closure raised meanwhile. The payload is this callback,
    /// which nothing else touches until `walk` returns; libgit2 must be done calling it by
    /// then.
    ///
    /// A walk that unwinds itself, rather than returning, leaves a panic of the closure
    /// behind; it belonged to that walk, so it is dropped here, before the walk it would
    /// otherwise skip the closure in and resume from.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_checkout_progress_cb, *mut c_void) -> R,
    ) -> R {
        self.panic = None;
        let result = walk(Some(progress_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            panic::resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for ProgressCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProgressCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_checkout_progress_cb` over a [`ProgressCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`ProgressCallback`] that nothing else uses during the
/// call, and `path` be NULL or a NUL-terminated string live and unchanged for the call.
unsafe extern "C" fn progress_with(
    path: *const c_char,
    completed_steps: usize,
    total_steps: usize,
    payload: *mut c_void,
) {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for the
    // call alone.
    let callback = unsafe { &mut *payload.cast::<ProgressCallback<'_>>() };
    if callback.panic.is_some() {
        return;
    }
    // SAFETY: per this function's contract, a non-NULL `path` is a NUL-terminated string
    // live and unchanged for the call; the borrow ends with it, since the closure is
    // generic over the string's lifetime.
    let path = (!path.is_null()).then(|| unsafe { CStr::from_ptr(path) });
    if let Err(panic) = panic::catch_unwind(AssertUnwindSafe(|| {
        (callback.f)(path, completed_steps, total_steps);
    })) {
        callback.panic = Some(panic);
    }
}

#[cfg(test)]
mod unit_tests {
    use std::ffi::CString;

    use super::*;
    use crate::libgit2::src::libgit2::index::{Index, IndexArc};
    use crate::libgit2::src::libgit2::strarray::{StrArray, StrArrayView};

    /// `GIT_CHECKOUT_OPTIONS_INIT`, built in Rust so Miri can run the accessors;
    /// `new_matches_the_template` ties it to `git_checkout_options_init`.
    fn template<'o>() -> CheckoutOptions<'o> {
        let raw = ffi::git_checkout_options {
            version: ffi::GIT_CHECKOUT_OPTIONS_VERSION,
            ..Default::default()
        };
        CheckoutOptions(raw, PhantomData)
    }

    /// Every field, as the getters read it.
    fn fields(o: CheckoutOptionsRef<'_, '_>) -> String {
        let (notify, notify_payload) = o.notify_callback();
        let (progress, progress_payload) = o.progress_callback();
        let (perfdata, perfdata_payload) = o.perfdata_callback();
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            o,
            notify.map(|f| f as usize),
            notify_payload,
            progress.map(|f| f as usize),
            progress_payload,
            perfdata.map(|f| f as usize),
            perfdata_payload,
            o.paths().iter().collect::<Vec<_>>(),
            (o.as_ptr() as usize) % align_of::<ffi::git_checkout_options>(),
        )
    }

    extern "C" fn notify(
        _: ffi::git_checkout_notify_t,
        _: *const c_char,
        _: *const ffi::git_diff_file,
        _: *const ffi::git_diff_file,
        _: *const ffi::git_diff_file,
        _: *mut c_void,
    ) -> c_int {
        0
    }

    extern "C" fn progress(_: *const c_char, _: usize, _: usize, _: *mut c_void) {}

    extern "C" fn perfdata(_: *const ffi::git_checkout_perfdata, _: *mut c_void) {}

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_the_template() {
        let options = CheckoutOptions::new();
        assert_eq!(fields(options.as_ref()), fields(template().as_ref()));
        assert_eq!(
            fields(CheckoutOptions::default().as_ref()),
            fields(options.as_ref())
        );
        let raw = options.0;
        assert_eq!(raw.version, 1);
        assert_eq!(
            (
                raw.checkout_strategy,
                raw.disable_filters,
                raw.dir_mode,
                raw.file_mode
            ),
            (0, 0, 0, 0)
        );
        assert_eq!((raw.file_open_flags, raw.notify_flags), (0, 0));
        assert!(raw.paths.strings.is_null() && raw.paths.count == 0);
        assert!(raw.baseline.is_null() && raw.baseline_index.is_null());
        assert!(raw.target_directory.is_null() && raw.ancestor_label.is_null());
        assert!(raw.our_label.is_null() && raw.their_label.is_null());
        assert!(raw.notify_cb.is_none() && raw.progress_cb.is_none() && raw.perfdata_cb.is_none());
        assert!(raw.notify_payload.is_null() && raw.progress_payload.is_null());
        assert!(raw.perfdata_payload.is_null());
    }

    #[test]
    fn defaults_read_as_documented() {
        let options = template();
        let o = options.as_ref();
        assert_eq!(o.version(), ffi::GIT_CHECKOUT_OPTIONS_VERSION);
        assert_eq!((o.strategy(), o.dir_mode(), o.file_mode()), (0, 0, 0));
        assert_eq!((o.file_open_flags(), o.notify_flags()), (0, 0));
        assert!(!o.disable_filters());
        assert!(o.paths().is_empty());
        assert!(o.baseline().is_none() && o.baseline_index().is_none());
        assert_eq!(o.target_directory(), None);
        assert_eq!(
            (o.ancestor_label(), o.our_label(), o.their_label()),
            (None, None, None)
        );
        assert!(o.notify_callback().0.is_none() && o.notify_callback().1.is_null());
        assert!(o.progress_callback().0.is_none() && o.progress_callback().1.is_null());
        assert!(o.perfdata_callback().0.is_none() && o.perfdata_callback().1.is_null());
    }

    #[test]
    fn scalar_setters_write_the_c_fields() {
        let mut options = template();
        let mut m = options.as_mut();
        m.set_strategy(0x8000_0012);
        m.set_disable_filters(true);
        m.set_dir_mode(0o700);
        m.set_file_mode(0o600);
        m.set_file_open_flags(-1);
        m.set_notify_flags(u32::MAX);
        let o = m.as_ref();
        assert_eq!(
            (o.strategy(), o.dir_mode(), o.file_mode()),
            (0x8000_0012, 0o700, 0o600)
        );
        assert_eq!((o.file_open_flags(), o.notify_flags()), (-1, u32::MAX));
        assert!(o.disable_filters());
        let raw = options.0;
        assert_eq!(
            (raw.checkout_strategy, raw.disable_filters),
            (0x8000_0012, 1)
        );
        assert_eq!((raw.dir_mode, raw.file_mode), (0o700, 0o600));
        assert_eq!((raw.file_open_flags, raw.notify_flags), (-1, u32::MAX));
        assert_eq!(raw.version, 1);

        options.as_mut().set_disable_filters(false);
        assert_eq!(options.0.disable_filters, 0);
        assert!(!options.as_ref().disable_filters());
    }

    #[test]
    fn disable_filters_reads_any_non_zero_int_as_set() {
        let mut options = template();
        options.0.disable_filters = -7;
        assert!(options.as_ref().disable_filters());
    }

    #[test]
    fn string_setters_borrow_and_clear() {
        let dir = CString::new("/tmp/out").unwrap();
        let (ancestor, ours, theirs) = (c"base", c"mine", c"yours");
        let mut options = template();
        let mut m = options.as_mut();
        m.set_target_directory(Some(&dir));
        m.set_ancestor_label(Some(ancestor));
        m.set_our_label(Some(ours));
        m.set_their_label(Some(theirs));
        let o = options.as_ref();
        assert_eq!(o.target_directory(), Some(dir.as_c_str()));
        assert_eq!(o.ancestor_label(), Some(ancestor));
        assert_eq!(o.our_label(), Some(ours));
        assert_eq!(o.their_label(), Some(theirs));
        // In place: the options point at the caller's strings, not at copies.
        assert_eq!(options.0.target_directory, dir.as_ptr());
        assert_eq!(options.0.our_label, ours.as_ptr());

        let mut m = options.as_mut();
        m.set_target_directory(None);
        m.set_ancestor_label(None);
        m.set_our_label(None);
        m.set_their_label(None);
        assert!(options.0.target_directory.is_null() && options.0.ancestor_label.is_null());
        assert!(options.0.our_label.is_null() && options.0.their_label.is_null());
    }

    #[test]
    fn string_getters_outlive_the_options() {
        let label = CString::new("mine").unwrap();
        let read = {
            let mut options = template();
            options.as_mut().set_our_label(Some(&label));
            options.as_ref().our_label()
        };
        assert_eq!(read, Some(label.as_c_str()));
    }

    #[test]
    fn set_paths_copies_a_view_header() {
        let strs = [c"*.txt", c"dir/b.txt"];
        let view = StrArrayView::new(&strs);
        let mut options = template();
        options.as_mut().set_paths(view.as_ref());
        let paths: Vec<_> = options.as_ref().paths().iter().collect();
        assert_eq!(paths, strs);
        // The options share the view's `char *` array.
        let shared = view.as_ref().as_ptr();
        // SAFETY: a read of the view's live `git_strarray`.
        let strings_ptr = unsafe { &raw const (*shared).strings };
        // SAFETY: as above.
        let strings = unsafe { strings_ptr.read() };
        assert_eq!(options.0.paths.strings, strings);
        assert_eq!(options.0.paths.count, 2);

        options.as_mut().clear_paths();
        assert!(options.0.paths.strings.is_null() && options.0.paths.count == 0);
        assert!(options.as_ref().paths().is_empty());
    }

    #[test]
    fn set_paths_stores_an_empty_array_as_null() {
        let view = StrArrayView::default();
        let mut options = template();
        options.as_mut().set_paths(view.as_ref());
        assert!(options.0.paths.strings.is_null() && options.0.paths.count == 0);

        // A NULL array with a stale count reads as empty, and is stored as such.
        let mut stale = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 3,
        };
        // SAFETY: `stale` is a live array, NULL, which the handles read as empty.
        let stale = unsafe { StrArrayRef::from_ptr(&raw mut stale) }.unwrap();
        options.as_mut().set_paths(stale);
        assert_eq!(options.0.paths.count, 0);
        assert!(options.as_ref().paths().is_empty());
    }

    #[test]
    fn set_paths_borrows_an_owned_array() {
        let mut owned = StrArray::new();
        // A non-NULL array with count 0, as libgit2's producers leave an empty result.
        let mut empty_buffer: Vec<*mut c_char> = vec![ptr::null_mut(); 1];
        let mut raw = ffi::git_strarray {
            strings: empty_buffer.as_mut_ptr(),
            count: 0,
        };
        // SAFETY: a live array of count 0; nothing is read through it.
        let empty = unsafe { StrArrayRef::from_ptr(&raw mut raw) }.unwrap();
        let mut options = template();
        options.as_mut().set_paths(empty);
        assert!(
            options.0.paths.strings.is_null(),
            "an empty array is not handed to C"
        );
        options.as_mut().set_paths(owned.as_ref());
        assert!(options.as_ref().paths().is_empty());
        // The options' last use ends their borrow of the owned array.
        owned.as_mut().truncate(0);
    }

    #[test]
    fn paths_getter_reads_the_embedded_header() {
        let strs = [c"a"];
        let view = StrArrayView::new(&strs);
        let mut options = template();
        options.as_mut().set_paths(view.as_ref());
        let o = options.as_ref();
        let field = &raw const options.0.paths;
        assert_eq!(o.paths().as_ptr(), field);
        assert_eq!(o.paths().get(0), Some(c"a"));
        assert_eq!(o.paths().get(1), None);
    }

    #[test]
    #[expect(
        clippy::many_single_char_names,
        reason = "short names for the sides and values the steps compare"
    )]
    fn raw_seams_round_trip() {
        let mut payloads = [0u8; 3];
        let [a, b, c] = payloads
            .each_mut()
            .map(|p| ptr::from_mut(p).cast::<c_void>());
        let mut options = template();
        let mut m = options.as_mut();
        // SAFETY: the callbacks ignore their arguments and are never called: no libgit2
        // routine sees these options; the payloads outlive them.
        unsafe { m.set_notify_callback(Some(notify), a) };
        // SAFETY: as above.
        unsafe { m.set_progress_callback(Some(progress), b) };
        // SAFETY: as above.
        unsafe { m.set_perfdata_callback(Some(perfdata), c) };
        let o = m.as_ref();
        let (n, np) = o.notify_callback();
        let (p, pp) = o.progress_callback();
        let (f, fp) = o.perfdata_callback();
        assert!(n.is_some() && p.is_some() && f.is_some());
        assert_eq!((np, pp, fp), (a, b, c));
        assert_eq!(options.0.notify_payload, a);
        assert_eq!(options.0.progress_payload, b);
        assert_eq!(options.0.perfdata_payload, c);

        let mut m = options.as_mut();
        // SAFETY: clearing stores nothing libgit2 would call or read.
        unsafe { m.set_notify_callback(None, ptr::null_mut()) };
        // SAFETY: as above.
        unsafe { m.set_progress_callback(None, ptr::null_mut()) };
        // SAFETY: as above.
        unsafe { m.set_perfdata_callback(None, ptr::null_mut()) };
        m.set_baseline(None);
        m.set_baseline_index(None);
        assert_eq!(fields(options.as_ref()), fields(template().as_ref()));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn baseline_index_stores_the_borrowed_pointer() {
        let index = Index::new().unwrap();
        let shared: IndexArc = Index::new().unwrap().into();
        let mut options = CheckoutOptions::new();
        options.as_mut().set_baseline_index(Some(index.as_ref()));
        assert_eq!(options.0.baseline_index, index.as_c_ptr());
        let read = options.as_ref().baseline_index().unwrap();
        assert_eq!(read.as_ptr(), index.as_ref().as_ptr());
        assert!(options.as_ref().baseline().is_none());
        options.as_mut().set_baseline_index(Some(shared.as_ref()));
        assert_eq!(
            options.as_ref().baseline_index().unwrap().as_ptr(),
            shared.as_ref().as_ptr()
        );
        options.as_mut().set_baseline_index(None);
        assert!(options.0.baseline_index.is_null());
        assert!(options.as_ref().baseline_index().is_none());
    }

    #[test]
    fn handles_reborrow_and_adopt_pointers() {
        let mut options = template();
        {
            let mut m = options.as_mut();
            m.as_mut().set_dir_mode(0o711);
            assert_eq!(m.as_ref().dir_mode(), 0o711);
            let p = m.as_mut_ptr();
            // SAFETY: `p` addresses `options`, live and reached through no other handle
            // while `n` lives; it stores no borrow.
            let mut n = unsafe { CheckoutOptionsMut::from_ptr(p) }.unwrap();
            n.set_file_mode(0o640);
        }
        assert_eq!(options.as_ref().file_mode(), 0o640);
        let p = ptr::from_mut(&mut options.0);
        // SAFETY: `p` addresses `options`, live and unwritten while `r` lives.
        let r = unsafe { CheckoutOptionsRef::from_ptr(p) }.unwrap();
        assert_eq!((r.dir_mode(), r.file_mode()), (0o711, 0o640));
        assert_eq!(r.as_ptr(), p.cast_const());
        // SAFETY: NULL is accepted, and gives no handle.
        assert!(unsafe { CheckoutOptionsRef::from_ptr(ptr::null_mut()) }.is_none());
        // SAFETY: as above.
        assert!(unsafe { CheckoutOptionsMut::from_ptr(ptr::null_mut()) }.is_none());
    }

    #[test]
    fn debug_shows_every_field() {
        let strs = [c"x"];
        let view = StrArrayView::new(&strs);
        let mut options = template();
        let mut m = options.as_mut();
        m.set_strategy(2);
        m.set_dir_mode(0o755);
        m.set_our_label(Some(c"mine"));
        m.set_paths(view.as_ref());
        let shown = format!("{m:?}");
        assert_eq!(shown, format!("{options:?}"));
        for field in [
            "version: 1",
            "strategy: 0x2",
            "disable_filters: false",
            "dir_mode: 0o755",
            "file_mode: 0o0",
            "file_open_flags: 0x0",
            "notify_flags: 0x0",
            "notify_callback: false",
            "progress_callback: false",
            "paths: [\"x\"]",
            "baseline: false",
            "baseline_index: false",
            "target_directory: None",
            "ancestor_label: None",
            "our_label: Some(\"mine\")",
            "their_label: None",
            "perfdata_callback: false",
        ] {
            assert!(shown.contains(field), "{field} missing from {shown}");
        }
    }

    #[test]
    fn layout_is_the_c_struct() {
        assert_eq!(
            size_of::<CheckoutOptions<'_>>(),
            size_of::<ffi::git_checkout_options>()
        );
        assert_eq!(
            size_of::<CheckoutOptionsRef<'_, '_>>(),
            size_of::<*const u8>()
        );
        assert_eq!(
            size_of::<CheckoutOptionsMut<'_, '_>>(),
            size_of::<*const u8>()
        );
        let options = template();
        assert_eq!(options.as_ref().as_ptr(), ptr::from_ref(&options.0));
    }

    fn counts(mkdir: usize, stat: usize, chmod: usize) -> CheckoutPerfdata {
        let mut d = CheckoutPerfdata::default();
        let mut m = d.as_mut();
        m.set_mkdir_calls(mkdir);
        m.set_stat_calls(stat);
        m.set_chmod_calls(chmod);
        d
    }

    #[test]
    fn perfdata_starts_at_zero_and_round_trips_every_counter() {
        let mut d = CheckoutPerfdata::zeroed();
        assert_eq!(d, CheckoutPerfdata::default());
        let r = d.as_ref();
        assert_eq!(
            (r.mkdir_calls(), r.stat_calls(), r.chmod_calls()),
            (0, 0, 0)
        );
        let mut m = d.as_mut();
        m.set_mkdir_calls(3);
        m.as_mut().set_stat_calls(usize::MAX);
        m.set_chmod_calls(1);
        let r = m.as_ref();
        assert_eq!(
            (r.mkdir_calls(), r.stat_calls(), r.chmod_calls()),
            (3, usize::MAX, 1)
        );
        // Each setter writes its own field: the C struct holds them in declaration order.
        assert_eq!(
            (d.0.mkdir_calls, d.0.stat_calls, d.0.chmod_calls),
            (3, usize::MAX, 1)
        );
    }

    #[test]
    fn perfdata_reads_a_c_struct_in_place() {
        let raw = ffi::git_checkout_perfdata {
            mkdir_calls: 2,
            stat_calls: 17,
            chmod_calls: 5,
        };
        // SAFETY: `raw` is a live, initialised `git_checkout_perfdata`, unwritten while the
        // handle borrows it, as a callback's argument is.
        let r = unsafe { CheckoutPerfdataRef::from_ptr(ptr::from_ref(&raw).cast_mut()) }.unwrap();
        assert_eq!(
            (r.mkdir_calls(), r.stat_calls(), r.chmod_calls()),
            (2, 17, 5)
        );
        assert_eq!(r.as_ptr(), ptr::from_ref(&raw));
        let copy = r.copied();
        assert_eq!(copy, counts(2, 17, 5));
        assert_ne!(ptr::from_ref(&copy).cast(), ptr::from_ref(&raw));
        // SAFETY: NULL borrows nothing.
        assert!(unsafe { CheckoutPerfdataRef::from_ptr(ptr::null_mut()) }.is_none());
    }

    #[test]
    fn perfdata_copies_compare_and_print_counter_by_counter() {
        let a = counts(1, 2, 3);
        let mut b = a;
        assert_eq!(a, b);
        b.as_mut().set_chmod_calls(4);
        assert_ne!(a, b);
        assert_eq!(a.as_ref().chmod_calls(), 3, "a copy is independent");
        assert_ne!(counts(9, 2, 3), a);
        assert_ne!(counts(1, 9, 3), a);
        let shown = "CheckoutPerfdata { mkdir_calls: 1, stat_calls: 2, chmod_calls: 3 }";
        assert_eq!(format!("{a:?}"), shown);
        assert_eq!(format!("{:?}", a.as_ref()), shown);
        let mut c = a;
        assert_eq!(format!("{:?}", c.as_mut()), shown);
    }

    #[test]
    fn perfdata_layout_is_the_c_struct() {
        assert_eq!(
            size_of::<CheckoutPerfdata>(),
            size_of::<ffi::git_checkout_perfdata>()
        );
        assert_eq!(size_of::<CheckoutPerfdataRef<'_>>(), size_of::<*const u8>());
        assert_eq!(size_of::<CheckoutPerfdataMut<'_>>(), size_of::<*const u8>());
    }

    #[test]
    fn perfdata_crosses_threads() {
        let a = counts(1, 2, 3);
        let r = a.as_ref();
        let read = std::thread::scope(|s| {
            let h = s.spawn(move || r.stat_calls());
            let h2 = s.spawn(|| a.as_ref().mkdir_calls());
            (h.join().unwrap(), h2.join().unwrap())
        });
        assert_eq!(read, (2, 1));
        let moved = std::thread::spawn(move || a.as_ref().chmod_calls())
            .join()
            .unwrap();
        assert_eq!(moved, 3);
    }

    /// The progress callback's tests.
    mod progress_callback {
        use std::path::PathBuf;
        use std::sync::OnceLock;
        use std::{fs, process};

        use super::*;

        /// `GIT_CHECKOUT_FORCE`, from `git2/checkout.h`: the `-sys` crate does not bind the
        /// strategy enum.
        const FORCE: u32 = 1 << 1;

        /// What a closure saw: the path (owned) and the two counts of each call.
        type Seen = Vec<(Option<CString>, usize, usize)>;

        /// A repository under a scratch directory, its working directory holding `files`,
        /// all staged in its index; released on drop.
        struct Staged {
            dir: PathBuf,
            repo: *mut ffi::git_repository,
            index: *mut ffi::git_index,
        }

        impl Staged {
            fn new(tag: &str, files: &[(&str, &str)]) -> Self {
                // libgit2 marks a filter initialised on its first load outside the registry
                // lock (`filter_initialize`), so concurrent first loads race (TSan): the
                // first fixture, whose staging loads the filters, is built alone.
                static PRIMED: OnceLock<()> = OnceLock::new();
                PRIMED.get_or_init(|| drop(Self::build("prime", FILES)));
                Self::build(tag, files)
            }

            fn build(tag: &str, files: &[(&str, &str)]) -> Self {
                // SAFETY: initialising libgit2 has no precondition; the reference is kept.
                assert!(unsafe { ffi::git_libgit2_init() } >= 1);
                let dir = std::env::temp_dir()
                    .join(format!("crustify-checkout-unit-{tag}-{}", process::id()));
                drop(fs::remove_dir_all(&dir));
                let work = dir.join("work");
                let c_work = CString::new(work.to_str().unwrap()).unwrap();
                let mut repo = ptr::null_mut();
                // SAFETY: a writable slot and a NUL-terminated path, read for the call; the
                // result is a repository this fixture owns.
                let rc = unsafe { ffi::git_repository_init(&raw mut repo, c_work.as_ptr(), 0) };
                assert_eq!(rc, 0);
                let mut index = ptr::null_mut();
                // SAFETY: a writable slot and the live repository; the result is one more
                // counted reference to its index, which this fixture releases.
                let rc = unsafe { ffi::git_repository_index(&raw mut index, repo) };
                assert_eq!(rc, 0);
                for (path, content) in files {
                    let file = work.join(path);
                    fs::create_dir_all(file.parent().unwrap()).unwrap();
                    fs::write(file, content).unwrap();
                    let path = CString::new(*path).unwrap();
                    // SAFETY: the live index, and a NUL-terminated path it copies.
                    let rc = unsafe { ffi::git_index_add_bypath(index, path.as_ptr()) };
                    assert_eq!(rc, 0);
                }
                Self { dir, repo, index }
            }

            /// Checks the index out into `target`, forced, with the callback `walk_with`
            /// lends, and returns libgit2's code.
            fn checkout(&self, target: &CStr, callback: &mut ProgressCallback<'_>) -> c_int {
                let mut options = CheckoutOptions::new();
                options.as_mut().set_strategy(FORCE);
                options.as_mut().set_target_directory(Some(target));
                callback.walk_with(|cb, payload| {
                    // SAFETY: the trampoline is sound for any arguments libgit2 passes, and
                    // `payload` stays valid until the walk returns, after the checkout,
                    // which is the options' only use.
                    unsafe { options.as_mut().set_progress_callback(cb, payload) };
                    // SAFETY: the live repository and index; live options, whose pointers
                    // are live or NULL, read for the call.
                    unsafe {
                        ffi::git_checkout_index(self.repo, self.index, options.as_ref().as_ptr())
                    }
                })
            }

            fn target(&self, name: &str) -> (PathBuf, CString) {
                let path = self.dir.join(name);
                let c = CString::new(path.to_str().unwrap()).unwrap();
                (path, c)
            }
        }

        impl Drop for Staged {
            fn drop(&mut self) {
                // SAFETY: the index reference `new` took, released once.
                unsafe { ffi::git_index_free(self.index) };
                // SAFETY: the repository `new` opened, freed once, after its index.
                unsafe { ffi::git_repository_free(self.repo) };
                drop(fs::remove_dir_all(&self.dir));
            }
        }

        const FILES: &[(&str, &str)] = &[
            ("a.txt", "one\n"),
            ("dir/b.txt", "two\n"),
            ("dir/sub/c.txt", "three\n"),
        ];

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn libgit2_reports_the_total_then_every_file() {
            let staged = Staged::new("report", FILES);
            let (path, target) = staged.target("out");
            let mut seen = Seen::new();
            let mut record = |path: Option<&CStr>, completed, total| {
                seen.push((path.map(CStr::to_owned), completed, total));
            };
            let mut callback = ProgressCallback::new(&mut record);
            assert_eq!(staged.checkout(&target, &mut callback), 0);
            drop(callback);
            assert_eq!(
                seen,
                [
                    (None, 0, 3),
                    (Some(c"a.txt".to_owned()), 1, 3),
                    (Some(c"dir/b.txt".to_owned()), 2, 3),
                    (Some(c"dir/sub/c.txt".to_owned()), 3, 3),
                ]
            );
            assert_eq!(fs::read(path.join("dir/sub/c.txt")).unwrap(), b"three\n");
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_panic_lets_the_checkout_finish_then_resumes() {
            let staged = Staged::new("panic", FILES);
            let (path, target) = staged.target("out");
            let mut calls = 0;
            let mut boom = |_: Option<&CStr>, completed, _| {
                calls += 1;
                assert!(completed < 1, "boom");
            };
            let mut callback = ProgressCallback::new(&mut boom);
            let caught =
                panic::catch_unwind(AssertUnwindSafe(|| staged.checkout(&target, &mut callback)));
            assert_eq!(*caught.unwrap_err().downcast::<&str>().unwrap(), "boom");
            assert!(format!("{callback:?}").contains("panicked: false"));
            drop(callback);
            // The baseline call and the one that panicked; the later two skip the closure.
            assert_eq!(calls, 2);
            // The callback cannot cancel: every file is written all the same.
            for (file, content) in FILES {
                assert_eq!(fs::read(path.join(file)).unwrap(), content.as_bytes());
            }

            // The same callback serves a later checkout, its earlier panic forgotten.
            let (_, again) = staged.target("again");
            let mut count = 0;
            let mut counting = |_: Option<&CStr>, _, _| count += 1;
            let mut callback = ProgressCallback::new(&mut counting);
            assert_eq!(staged.checkout(&again, &mut callback), 0);
            drop(callback);
            assert_eq!(count, 4);
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_dry_run_never_calls_back() {
            const DRY_RUN: u32 = 1 << 24;
            let staged = Staged::new("dry", FILES);
            let (_, target) = staged.target("out");
            let mut calls = 0;
            let mut count = |_: Option<&CStr>, _, _| calls += 1;
            let mut callback = ProgressCallback::new(&mut count);
            let mut options = CheckoutOptions::new();
            options.as_mut().set_strategy(FORCE | DRY_RUN);
            options.as_mut().set_target_directory(Some(&target));
            let rc = callback.walk_with(|cb, payload| {
                // SAFETY: as in `Staged::checkout`.
                unsafe { options.as_mut().set_progress_callback(cb, payload) };
                // SAFETY: as in `Staged::checkout`.
                unsafe {
                    ffi::git_checkout_index(staged.repo, staged.index, options.as_ref().as_ptr())
                }
            });
            drop(callback);
            assert_eq!((rc, calls), (0, 0));
        }

        #[test]
        fn call_lends_the_path_and_the_counts() {
            let mut seen = Seen::new();
            let mut record = |path: Option<&CStr>, completed, total| {
                seen.push((path.map(CStr::to_owned), completed, total));
            };
            let mut callback = ProgressCallback::new(&mut record);
            callback.call(None, 0, usize::MAX);
            let owned = CString::new("dir/é.txt").unwrap();
            callback.call(Some(&owned), usize::MAX, usize::MAX);
            callback.call(Some(c""), 7, 3);
            assert_eq!(
                format!("{callback:?}"),
                "ProgressCallback { panicked: false, .. }"
            );
            drop(callback);
            assert_eq!(
                seen,
                [
                    (None, 0, usize::MAX),
                    (Some(owned), usize::MAX, usize::MAX),
                    (Some(c"".to_owned()), 7, 3),
                ]
            );
        }

        #[test]
        fn the_trampoline_reads_a_null_path_as_none() {
            let mut seen = Vec::new();
            let mut record = |path: Option<&CStr>, completed, total| {
                seen.push((path.is_none(), completed, total));
            };
            let mut callback = ProgressCallback::new(&mut record);
            callback.walk_with(|cb, payload| {
                let cb = cb.unwrap();
                // SAFETY: `payload` is the callback `walk_with` lends; NULL is in the
                // trampoline's contract.
                unsafe { cb(ptr::null(), 0, 2, payload) };
                // SAFETY: as above, with a string live for the call.
                unsafe { cb(c"x".as_ptr(), 1, 2, payload) };
            });
            drop(callback);
            assert_eq!(seen, [(true, 0, 2), (false, 1, 2)]);
        }

        #[test]
        fn a_panic_skips_later_calls_of_the_walk_then_resumes() {
            let mut calls = 0;
            let mut boom = |_: Option<&CStr>, _, _| {
                calls += 1;
                panic!("boom");
            };
            let mut callback = ProgressCallback::new(&mut boom);
            let mut after = false;
            let caught = panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    let cb = cb.unwrap();
                    for step in 0..3 {
                        // SAFETY: `payload` is the callback `walk_with` lends.
                        unsafe { cb(ptr::null(), step, 3, payload) };
                    }
                    // Control comes back to the walk, which runs to its end.
                    after = true;
                });
            }));
            assert_eq!(*caught.unwrap_err().downcast::<&str>().unwrap(), "boom");
            assert!(after);
            assert!(format!("{callback:?}").contains("panicked: false"));
            drop(callback);
            assert_eq!(calls, 1);
        }

        #[test]
        fn call_resumes_a_panic_and_the_callback_runs_again() {
            let mut calls = 0;
            let mut f = |_: Option<&CStr>, completed: usize, _| {
                calls += 1;
                assert_ne!(completed, 1, "one");
            };
            let mut callback = ProgressCallback::new(&mut f);
            callback.call(None, 0, 2);
            let caught = panic::catch_unwind(AssertUnwindSafe(|| callback.call(None, 1, 2)));
            assert!(caught.is_err());
            callback.call(None, 2, 2);
            drop(callback);
            assert_eq!(calls, 3);
        }

        #[test]
        fn a_walk_that_unwinds_leaves_no_stale_panic() {
            let mut calls = 0;
            let mut f = |_: Option<&CStr>, _, _| {
                calls += 1;
                assert!(calls > 1, "first call");
            };
            let mut callback = ProgressCallback::new(&mut f);
            // The closure panics inside the walk, which then unwinds on its own before
            // `walk_with` could resume the closure's panic.
            let walked = panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    // SAFETY: `payload` is the callback `walk_with` lends.
                    unsafe { cb.unwrap()(ptr::null(), 0, 1, payload) };
                    panic!("the walk's own");
                })
            }));
            assert_eq!(
                *walked.unwrap_err().downcast::<&str>().unwrap(),
                "the walk's own"
            );
            // The next walk runs the closure: the first walk's panic was dropped.
            callback.call(None, 1, 1);
            drop(callback);
            assert_eq!(calls, 2);
        }
    }
}
