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
//! holding it. The notification, progress and performance-data callbacks are a
//! [`NotifyCallback`], a [`ProgressCallback`] and a [`PerfdataCallback`], each borrowed
//! exclusively for `'o`: the options store its trampoline as the C callback and the
//! callback itself as its payload, and report what fills each slot as a [`CallbackSlot`],
//! never as a raw pointer. A panic of a callback's closure resumes from the wrapper of the
//! routine the options were passed to, once libgit2 has returned. Options C filled in,
//! or embedded in a struct C filled in, may hold C callbacks no Rust wrapper stored:
//! they are reported as [`CallbackSlot::Foreign`] and left as they are, and storing C
//! callbacks for C interop goes through the options' raw seam
//! ([`CMutSeam::as_mut_ptr`]). The flags fields take the
//! raw bits of `git_checkout_strategy_t` and `git_checkout_notify_t` until those are wrapped.
//!
//! `git_rebase_init` and `git_rebase_open` are the one exception to "read during the
//! call": the rebase keeps a copy of the options, duplicating the strings and the
//! baseline tree but keeping the `paths` array, the callbacks and payloads, and the
//! baseline index as given, and `git_rebase_free` releases that index without having
//! taken a reference to it. Their wrappers must keep what the copy borrows alive for
//! the rebase, Rust callbacks included, resume those callbacks' panics after each call
//! that may run them, and must not hand them a baseline index.
//!
//! # Performance data
//!
//! A checkout that has written the working directory reports the filesystem calls it made to
//! its performance-data callback as a `git_checkout_perfdata`: three counters, zeroed when the
//! checkout starts, held in its own state and lent to the callback for the call. It reports
//! before it writes the index, so a checkout whose index write then fails has still reported:
//! a report does not mean the checkout succeeded. Rust reads them through a
//! [`CheckoutPerfdataRef`], and holds a copy, or a value of its own, as a
//! [`CheckoutPerfdata`].
//!
//! # Progress callbacks
//!
//! A [`ProgressCallback`] is a `git_checkout_progress_cb` over a Rust closure. A checkout
//! calls one synchronously, on its own thread, while it runs: once to report how many
//! steps it will take, then once per file it removes or writes and per conflict it
//! writes, lending it the path for the call alone. The conflicts it removes from the index
//! are steps it counts but does not report. The callback returns nothing, so it
//! observes the checkout without steering it. The options'
//! [`set_progress_callback`](CheckoutOptionsMut::set_progress_callback) stores it, and the
//! wrappers of the routines that check out with them resume a panic its closure raised
//! once C has returned.
//!
//! # Notification callbacks
//!
//! A [`NotifyCallback`] is a `git_checkout_notify_cb` over a Rust closure. A checkout calls
//! one synchronously, on its own thread, while it plans and before it writes anything: for
//! each path in a situation its [`notify_flags`](CheckoutOptionsRef::notify_flags) select,
//! sometimes more than once for one path, lending it the path and the baseline, target and
//! working-directory sides of it, as [`DiffFileRef`]s, for the call alone. The closure returns
//! [`ControlFlow::Break`] to cancel, which the callback reports as `GIT_EUSER`, the code the
//! checkout then fails with; a panic cancels the same way and resumes once C has returned.
//!
//! # Performance-data callbacks
//!
//! A [`PerfdataCallback`] is a `git_checkout_perfdata_cb` over a Rust closure, which a
//! checkout calls once, synchronously, when it has written everything, lending it its
//! counters as a [`CheckoutPerfdataRef`] for the call. Like the progress callback, it
//! returns nothing and cannot fail the checkout.
//!
//! The options' [`set_notify_callback`](CheckoutOptionsMut::set_notify_callback) and
//! [`set_perfdata_callback`](CheckoutOptionsMut::set_perfdata_callback) store both as
//! [`set_progress_callback`](CheckoutOptionsMut::set_progress_callback) stores a
//! [`ProgressCallback`]. A closure that reaches the options it is stored in, through
//! interior mutability, and checks out with them again finds its callback busy: the
//! nested checkout is cancelled by a notification callback, and reports nothing to a
//! progress or performance-data callback.

use core::any::Any;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::ControlFlow;
use core::ptr::{self};
use std::panic::{self, AssertUnwindSafe};

use ffibox::{CBorrowedPtr, CCell, CMutSeam, CRefSeam, CZeroable, define_ctype};

use crate::ffi;
use crate::libgit2::src::libgit2::diff::DiffFileRef;
use crate::libgit2::src::libgit2::index::IndexRef;
use crate::libgit2::src::libgit2::strarray::StrArrayRef;
use crate::libgit2::src::libgit2::tree::TreeRef;
use crate::libgit2::src::util::errors::ErrorCode;

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
/// Neither `Send` nor `Sync`: the callbacks it stores borrow closures that need be
/// neither, and the C callbacks options C filled in may hold carry payloads whose thread
/// affinity the caller alone knows.
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;
///
/// let options = CheckoutOptions::new();
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Not `Clone`: the callbacks it stores are exclusive borrows, which a copy would
/// alias. Build another with [`new`](Self::new) instead.
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

    /// What fills the notification callback: [`CallbackSlot::Empty`] by default,
    /// [`CallbackSlot::Rust`] for a [`NotifyCallback`]
    /// [`set_notify_callback`](CheckoutOptionsMut::set_notify_callback) stored, and
    /// [`CallbackSlot::Foreign`] for a C callback stored by C, or through the raw seam, whose
    /// function and payload the options cannot type and so do not hand out.
    ///
    /// Wraps `fld::git_checkout_options.notify_cb`
    /// Wraps `fld::git_checkout_options.notify_payload`
    #[doc(alias = "git_checkout_options.notify_cb")]
    #[doc(alias = "git_checkout_options.notify_payload")]
    #[must_use]
    pub fn notify_callback(&self) -> CallbackSlot {
        let (callback, payload) = self.notify_fields();
        CallbackSlot::of(
            callback.is_some(),
            stored_notify(callback, payload).is_some(),
        )
    }

    /// What fills the progress callback, as
    /// [`notify_callback`](Self::notify_callback) reports the notification callback: a
    /// [`ProgressCallback`] [`set_progress_callback`](CheckoutOptionsMut::set_progress_callback)
    /// stored is [`CallbackSlot::Rust`].
    ///
    /// Wraps `fld::git_checkout_options.progress_cb`
    /// Wraps `fld::git_checkout_options.progress_payload`
    #[doc(alias = "git_checkout_options.progress_cb")]
    #[doc(alias = "git_checkout_options.progress_payload")]
    #[must_use]
    pub fn progress_callback(&self) -> CallbackSlot {
        let (callback, payload) = self.progress_fields();
        CallbackSlot::of(
            callback.is_some(),
            stored_progress(callback, payload).is_some(),
        )
    }

    /// What fills the performance-data callback, as
    /// [`notify_callback`](Self::notify_callback) reports the notification callback: a
    /// [`PerfdataCallback`] [`set_perfdata_callback`](CheckoutOptionsMut::set_perfdata_callback)
    /// stored is [`CallbackSlot::Rust`].
    ///
    /// Wraps `fld::git_checkout_options.perfdata_cb`
    /// Wraps `fld::git_checkout_options.perfdata_payload`
    #[doc(alias = "git_checkout_options.perfdata_cb")]
    #[doc(alias = "git_checkout_options.perfdata_payload")]
    #[must_use]
    pub fn perfdata_callback(&self) -> CallbackSlot {
        let (callback, payload) = self.perfdata_fields();
        CallbackSlot::of(
            callback.is_some(),
            stored_perfdata(callback, payload).is_some(),
        )
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

    /// Runs `walk` with the pointer to these options, as the wrapper of a routine that
    /// checks out with them hands it to C (`git_checkout_head`, `git_checkout_index`,
    /// `git_checkout_tree`, the routines that check out through them, and
    /// `git_worktree_add` for the options its own embed), then resumes a panic that a Rust
    /// callback stored in them raised meanwhile. The notification callback's panic resumes
    /// first, then the progress callback's, then the performance-data callback's; any
    /// other is dropped. Callbacks C stored are left alone.
    ///
    /// A panic left behind by an earlier call, which no wrapper resumed (the options were
    /// handed to C directly), is dropped first: it belonged to that call. A walk that a
    /// stored callback's closure starts, with these same options, while that callback is
    /// busy, is nested in the walk running the closure, and leaves the panics that walk's
    /// callbacks already raised in place: the outer walk resumes them, and the nested
    /// checkout skips their closures, as the rest of the outer one does.
    pub(crate) fn walk_with<R>(
        self,
        walk: impl FnOnce(*const ffi::git_checkout_options) -> R,
    ) -> R {
        let slots = self.stored_slots();
        let nested = slots.iter().flatten().any(|&(_, busy)| {
            // SAFETY: `busy` is the busy flag of a live callback the options borrow
            // exclusively for `'o`, on this thread; only raw pointers reach it, and its
            // trampoline holds no reference to it across its closure.
            unsafe { busy.read() }
        });
        let mut outer = [false; 3];
        for (held, slot) in outer.iter_mut().zip(slots) {
            let Some((panic, _)) = slot else { continue };
            if nested {
                // SAFETY: `panic` is the panic slot of a live callback the options borrow
                // exclusively for `'o`, on this thread (neither is `Send`). Only raw
                // pointers reach the slot, and its trampoline holds none across its
                // closure, which is the one place this could run during a trampoline: no
                // reference to it is live. The reference `is_some` makes ends with the
                // statement.
                *held = unsafe { (*panic).is_some() };
            } else {
                // SAFETY: as above.
                unsafe { *panic = None };
            }
        }
        let result = walk(self.as_ptr());
        let mut first = None;
        for (held, slot) in outer.into_iter().zip(self.stored_slots()) {
            let Some((panic, _)) = slot else { continue };
            if held {
                continue;
            }
            // SAFETY: as above; the reference `take` makes ends with the statement.
            let taken = unsafe { (*panic).take() };
            first = first.or(taken);
        }
        if let Some(panic) = first {
            panic::resume_unwind(panic);
        }
        result
    }

    /// The panic slots and busy flags of the Rust callbacks stored in the options, in the
    /// order their panics resume.
    fn stored_slots(self) -> [Option<(*mut Option<Panic>, *mut bool)>; 3] {
        let (notify, notify_payload) = self.notify_fields();
        let (progress, progress_payload) = self.progress_fields();
        let (perfdata, perfdata_payload) = self.perfdata_fields();
        // Each callback is the live one its setter stored and the options borrow for `'o`,
        // which outlives this handle.
        [
            stored_notify(notify, notify_payload).map(|callback| {
                // SAFETY: `callback` is live, as above; the projection forms no reference.
                let panic = unsafe { &raw mut (*callback).panic };
                // SAFETY: as above.
                let busy = unsafe { &raw mut (*callback).busy };
                (panic, busy)
            }),
            stored_progress(progress, progress_payload).map(|callback| {
                // SAFETY: as above.
                let panic = unsafe { &raw mut (*callback).panic };
                // SAFETY: as above.
                let busy = unsafe { &raw mut (*callback).busy };
                (panic, busy)
            }),
            stored_perfdata(perfdata, perfdata_payload).map(|callback| {
                // SAFETY: as above.
                let panic = unsafe { &raw mut (*callback).panic };
                // SAFETY: as above.
                let busy = unsafe { &raw mut (*callback).busy };
                (panic, busy)
            }),
        ]
    }

    /// The notification callback's fields, as stored.
    fn notify_fields(self) -> (ffi::git_checkout_notify_cb, *mut c_void) {
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

    /// The progress callback's fields, as stored.
    fn progress_fields(self) -> (ffi::git_checkout_progress_cb, *mut c_void) {
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

    /// The performance-data callback's fields, as stored.
    fn perfdata_fields(self) -> (ffi::git_checkout_perfdata_cb, *mut c_void) {
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
}

impl fmt::Debug for CheckoutOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            .field("notify_callback", &self.notify_callback())
            .field("progress_callback", &self.progress_callback())
            .field("paths", &self.paths())
            .field("baseline", &self.baseline().is_some())
            .field("baseline_index", &self.baseline_index().is_some())
            .field("target_directory", &self.target_directory())
            .field("ancestor_label", &self.ancestor_label())
            .field("our_label", &self.our_label())
            .field("their_label", &self.their_label())
            .field("perfdata_callback", &self.perfdata_callback())
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

    /// Sets the notification callback, which the options then borrow exclusively for
    /// `'o`; `None` calls nothing. Every libgit2 call the options are passed to calls it,
    /// for the situations in [`notify_flags`](CheckoutOptionsRef::notify_flags), before
    /// it changes any file; a [`ControlFlow::Break`], or a panic, of its closure cancels
    /// the checkout, which fails with `GIT_EUSER`. The panic resumes from the wrapper of
    /// the routine the options were passed to, once libgit2 has returned.
    ///
    /// The options store the callback's trampoline as `notify_cb` and the callback itself
    /// as `notify_payload`, replacing whatever was there, a C callback included. The
    /// callback cannot be used, moved or dropped while the options may still call it, and
    /// a checkout its closure starts with these same options (which only interior
    /// mutability lets it reach) finds it busy and is cancelled.
    ///
    /// ```
    /// use std::ops::ControlFlow;
    ///
    /// use libgit2::libgit2::src::libgit2::checkout::{
    ///     CallbackSlot, CheckoutNotification, CheckoutOptions, NotifyCallback,
    /// };
    ///
    /// # if cfg!(miri) { return; }
    /// let mut paths = Vec::new();
    /// let mut record = |n: CheckoutNotification<'_>| {
    ///     paths.extend(n.path.map(ToOwned::to_owned));
    ///     ControlFlow::Continue(())
    /// };
    /// let mut callback = NotifyCallback::new(&mut record);
    /// let mut options = CheckoutOptions::new();
    /// options.as_mut().set_notify_callback(Some(&mut callback));
    /// assert_eq!(options.as_ref().notify_callback(), CallbackSlot::Rust);
    /// options.as_mut().set_notify_callback(None);
    /// assert_eq!(options.as_ref().notify_callback(), CallbackSlot::Empty);
    /// ```
    ///
    /// Wraps `fld::git_checkout_options.notify_cb`
    /// Wraps `fld::git_checkout_options.notify_payload`
    #[doc(alias = "git_checkout_options.notify_cb")]
    #[doc(alias = "git_checkout_options.notify_payload")]
    pub fn set_notify_callback(&mut self, callback: Option<&'o mut NotifyCallback<'_>>) {
        let (notify_cb, notify_payload): (ffi::git_checkout_notify_cb, *mut c_void) = match callback
        {
            Some(callback) => {
                callback.panic = None;
                (Some(notify_with), ptr::from_mut(callback).cast())
            }
            None => (None, ptr::null_mut()),
        };
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let notify_cb_ptr = unsafe { &raw mut (*p).notify_cb };
        // SAFETY: as above.
        let notify_payload_ptr = unsafe { &raw mut (*p).notify_payload };
        // SAFETY: the fields are plain data, so the old values need no drop. The trampoline
        // is sound for any arguments libgit2 passes with this payload, a callback borrowed
        // exclusively for `'o`, which the options cannot outlive and do not share across
        // threads (they are neither `Send` nor `Sync`, nor `Clone`).
        unsafe { notify_cb_ptr.write(notify_cb) };
        // SAFETY: as above.
        unsafe { notify_payload_ptr.write(notify_payload) };
    }

    /// Sets the progress callback, which the options then borrow exclusively for `'o`;
    /// `None` calls nothing. Every libgit2 call the options are passed to reports to it
    /// the total, then each file it writes or removes. A panic of its closure skips the
    /// closure for the rest of the checkout and resumes from the wrapper of the routine
    /// the options were passed to, once libgit2 has returned.
    ///
    /// Stored as [`set_notify_callback`](Self::set_notify_callback) stores a notification
    /// callback, as `progress_cb` and `progress_payload`; a checkout its closure starts
    /// with these same options finds it busy and reports nothing to it.
    ///
    /// Wraps `fld::git_checkout_options.progress_cb`
    /// Wraps `fld::git_checkout_options.progress_payload`
    #[doc(alias = "git_checkout_options.progress_cb")]
    #[doc(alias = "git_checkout_options.progress_payload")]
    pub fn set_progress_callback(&mut self, callback: Option<&'o mut ProgressCallback<'_>>) {
        let (progress_cb, progress_payload): (ffi::git_checkout_progress_cb, *mut c_void) =
            match callback {
                Some(callback) => {
                    callback.panic = None;
                    (Some(progress_with), ptr::from_mut(callback).cast())
                }
                None => (None, ptr::null_mut()),
            };
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let progress_cb_ptr = unsafe { &raw mut (*p).progress_cb };
        // SAFETY: as above.
        let progress_payload_ptr = unsafe { &raw mut (*p).progress_payload };
        // SAFETY: as in `set_notify_callback`, for the progress trampoline and callback.
        unsafe { progress_cb_ptr.write(progress_cb) };
        // SAFETY: as above.
        unsafe { progress_payload_ptr.write(progress_payload) };
    }

    /// Sets the performance-data callback, which the options then borrow exclusively for
    /// `'o`; `None` calls nothing. Every libgit2 call the options are passed to calls it
    /// once it has written the working directory, before it writes the index, which can
    /// still fail the checkout, with the counts of the `mkdir`, `stat` and `chmod` calls
    /// it made. A panic of its closure resumes from the wrapper of the routine the options
    /// were passed to, once libgit2 has returned.
    ///
    /// Stored as [`set_notify_callback`](Self::set_notify_callback) stores a notification
    /// callback, as `perfdata_cb` and `perfdata_payload`; a checkout its closure starts
    /// with these same options finds it busy and reports nothing to it.
    ///
    /// Wraps `fld::git_checkout_options.perfdata_cb`
    /// Wraps `fld::git_checkout_options.perfdata_payload`
    #[doc(alias = "git_checkout_options.perfdata_cb")]
    #[doc(alias = "git_checkout_options.perfdata_payload")]
    pub fn set_perfdata_callback(&mut self, callback: Option<&'o mut PerfdataCallback<'_>>) {
        let (perfdata_cb, perfdata_payload): (ffi::git_checkout_perfdata_cb, *mut c_void) =
            match callback {
                Some(callback) => {
                    callback.panic = None;
                    (Some(perfdata_with), ptr::from_mut(callback).cast())
                }
                None => (None, ptr::null_mut()),
            };
        let p = self.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options; the projection forms no
        // reference to them.
        let perfdata_cb_ptr = unsafe { &raw mut (*p).perfdata_cb };
        // SAFETY: as above.
        let perfdata_payload_ptr = unsafe { &raw mut (*p).perfdata_payload };
        // SAFETY: as in `set_notify_callback`, for the performance-data trampoline and
        // callback.
        unsafe { perfdata_cb_ptr.write(perfdata_cb) };
        // SAFETY: as above.
        unsafe { perfdata_payload_ptr.write(perfdata_payload) };
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

/// What fills one of a [`CheckoutOptions`]' callbacks, as its getters
/// ([`notify_callback`](CheckoutOptionsRef::notify_callback),
/// [`progress_callback`](CheckoutOptionsRef::progress_callback),
/// [`perfdata_callback`](CheckoutOptionsRef::perfdata_callback)) report it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallbackSlot {
    /// No callback: libgit2 calls nothing, whatever the payload holds.
    Empty,
    /// A Rust callback the options' safe setter stored, which they borrow until they are
    /// dropped or the callback is set again.
    Rust,
    /// A C callback stored by C, or through the options' raw seam, with a payload the
    /// options cannot type. Options C filled in, or embedded in a struct C filled in, can
    /// hold one.
    Foreign,
}

impl CallbackSlot {
    fn of(set: bool, stored_by_rust: bool) -> Self {
        match (set, stored_by_rust) {
            (_, true) => Self::Rust,
            (true, false) => Self::Foreign,
            (false, false) => Self::Empty,
        }
    }
}

/// A panic a callback's closure raised inside C, resumed once control is back in Rust.
type Panic = Box<dyn Any + Send + 'static>;

// A slot holds a Rust callback when it holds the trampoline the safe setter stores, with a
// payload. The trampolines are private, non-generic and never inlined, so each has one
// address, and only this module stores them, always with their callback as the payload: a
// foreign function cannot share a trampoline's address, so no foreign payload is taken for
// a callback. The lifetime of the callback found is erased: only its panic slot is
// reached through it, never its closure.

/// The [`NotifyCallback`] a notification slot holds, when the safe setter stored one.
fn stored_notify(
    callback: ffi::git_checkout_notify_cb,
    payload: *mut c_void,
) -> Option<*mut NotifyCallback<'static>> {
    let ours: ffi::git_checkout_notify_cb = Some(notify_with);
    let stored = callback
        .zip(ours)
        .is_some_and(|(f, g)| ptr::fn_addr_eq(f, g));
    (stored && !payload.is_null()).then(|| payload.cast())
}

/// The [`ProgressCallback`] a progress slot holds, when the safe setter stored one.
fn stored_progress(
    callback: ffi::git_checkout_progress_cb,
    payload: *mut c_void,
) -> Option<*mut ProgressCallback<'static>> {
    let ours: ffi::git_checkout_progress_cb = Some(progress_with);
    let stored = callback
        .zip(ours)
        .is_some_and(|(f, g)| ptr::fn_addr_eq(f, g));
    (stored && !payload.is_null()).then(|| payload.cast())
}

/// The [`PerfdataCallback`] a performance-data slot holds, when the safe setter stored one.
fn stored_perfdata(
    callback: ffi::git_checkout_perfdata_cb,
    payload: *mut c_void,
) -> Option<*mut PerfdataCallback<'static>> {
    let ours: ffi::git_checkout_perfdata_cb = Some(perfdata_with);
    let stored = callback
        .zip(ours)
        .is_some_and(|(f, g)| ptr::fn_addr_eq(f, g));
    (stored && !payload.is_null()).then(|| payload.cast())
}

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
/// step, to report the total, then once after each file it removes or writes and each
/// conflict it writes out, with that path, relative to the working directory, and the count
/// so far.
/// The path is lent for the call only (it is the diff's or the checkout's own string), so
/// the closure receives it as a `&CStr` that cannot outlive the call. A dry run, or a
/// checkout that stops before it writes anything, never calls it.
///
/// Not every step is reported. A checkout that removes stale conflicts from the index
/// counts each removal as a step, in the total and in the completed count, but calls back
/// for none of them: the removals come first, so the next call's count skips them, and a
/// checkout that only removes conflicts calls back once, for the baseline. The last call
/// therefore need not report every step completed.
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
    panic: Option<Panic>,
    /// Whether the closure is running: a call that comes meanwhile, from a checkout the
    /// closure started with the options that store this callback, does not run it again.
    busy: bool,
}

impl<'f> ProgressCallback<'f> {
    /// The callback that calls `f` with each path and step count.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(Option<&CStr>, usize, usize) + 'f,
    {
        Self {
            f,
            panic: None,
            busy: false,
        }
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

    /// Runs `walk` with the C callback and its payload, as [`call`](Self::call) passes them
    /// to the trampoline, or C interop stores them for a call that cannot go through the
    /// options' [setter](CheckoutOptionsMut), then resumes a panic the closure raised
    /// meanwhile. The payload is this callback, which nothing else touches until `walk`
    /// returns; libgit2 must be done calling it by then.
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
/// `payload` must address a live [`ProgressCallback`] that only this trampoline's calls use
/// meanwhile, on this thread, and `path` be NULL or a NUL-terminated string live and
/// unchanged for the call.
#[inline(never)]
unsafe extern "C" fn progress_with(
    path: *const c_char,
    completed_steps: usize,
    total_steps: usize,
    payload: *mut c_void,
) {
    let callback = payload.cast::<ProgressCallback<'_>>();
    // SAFETY: per this function's contract, `payload` addresses a live callback; the
    // projections form no reference to it, so a nested call, from its closure, aliases none.
    let busy = unsafe { &raw mut (*callback).busy };
    // SAFETY: as above.
    let panic = unsafe { &raw mut (*callback).panic };
    // SAFETY: a read of the live callback's field, on its thread; no reference to it is live.
    let busy_now = unsafe { busy.read() };
    // SAFETY: as above; the reference `is_some` makes ends with the statement.
    let panicked = unsafe { (*panic).is_some() };
    if busy_now || panicked {
        return;
    }
    // SAFETY: per this function's contract, a non-NULL `path` is a NUL-terminated string
    // live and unchanged for the call; the borrow ends with it, since the closure is
    // generic over the string's lifetime.
    let path = (!path.is_null()).then(|| unsafe { CStr::from_ptr(path) });
    // SAFETY: the closure the live callback borrows. Until `busy` is cleared, no other call
    // of this trampoline reaches it, and nothing else does while the callback is lent to
    // C: the options or `walk_with` borrow it exclusively.
    let closure = unsafe { &mut *(*callback).f };
    // SAFETY: a write to the live callback's field; no reference to it is live.
    unsafe { busy.write(true) };
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        closure(path, completed_steps, total_steps);
    }));
    // SAFETY: as above.
    unsafe { busy.write(false) };
    if let Err(payload) = result {
        // SAFETY: as above; the slot is `None`, as checked, and nothing ran the closure
        // meanwhile to fill it.
        unsafe { *panic = Some(payload) };
    }
}

/// What a [`NotifyCallback`] is told about one path, lent for the call: the arguments of a
/// `git_checkout_notify_cb` but its payload.
///
/// Every borrow is valid for `'a`, the call: the path and the files belong to the
/// checkout's diff and iterators, and the workdir file is a copy on its stack.
#[derive(Clone, Copy)]
pub struct CheckoutNotification<'a> {
    /// The one `GIT_CHECKOUT_NOTIFY_*` bit naming the situation: a conflict, a dirty file,
    /// an update, an untracked or an ignored file.
    pub why: u32,
    /// The path, relative to the working directory, with a trailing `/` for a directory.
    /// libgit2 always passes one; a NULL path is read as `None` rather than invented.
    pub path: Option<&'a CStr>,
    /// The baseline's side of a change the checkout plans: the file the working directory
    /// was expected to hold. `None` for a path the baseline does not have, and for a
    /// notification about the working directory alone (an untracked or ignored item, or a
    /// dirty file outside the checkout's changes).
    pub baseline: Option<DiffFileRef<'a>>,
    /// The target's side of a change the checkout plans: the file it checks out. `None`
    /// for a path the target deletes, and for a notification about the working directory
    /// alone.
    pub target: Option<DiffFileRef<'a>>,
    /// What the working directory holds at the path: a file, or a directory (mode
    /// `GIT_FILEMODE_TREE`). `None` when nothing is there, and also when a directory
    /// stands where the baseline and the target have the same file: libgit2 then notifies
    /// that file as dirty without this side, before it notifies the directory.
    pub workdir: Option<DiffFileRef<'a>>,
}

impl fmt::Debug for CheckoutNotification<'_> {
    /// Each side by its path; a side without one as the empty path.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn side(file: Option<DiffFileRef<'_>>) -> Option<&CStr> {
            file.map(|file| file.path().unwrap_or_default())
        }
        f.debug_struct("CheckoutNotification")
            .field("why", &format_args!("{:#x}", self.why))
            .field("path", &self.path)
            .field("baseline", &side(self.baseline))
            .field("target", &side(self.target))
            .field("workdir", &side(self.workdir))
            .finish()
    }
}

/// The closure behind a [`NotifyCallback`].
type NotifyFn<'f> = dyn FnMut(CheckoutNotification<'_>) -> ControlFlow<()> + 'f;

/// A `git_checkout_notify_cb`: the callback a checkout calls, before it changes any file,
/// for each path in one of the situations its options'
/// [`notify_flags`](CheckoutOptionsRef::notify_flags) select, and which may cancel it.
///
/// Built over a closure with [`new`](Self::new). libgit2 calls it synchronously, on the
/// thread running the checkout, while it plans what to do and before it writes anything
/// (a dry run plans, so it notifies too):
/// for each conflicting, dirty, updated, untracked or ignored path it meets, with a
/// [`CheckoutNotification`]: the `GIT_CHECKOUT_NOTIFY_*` bit naming the situation, the
/// path, and the sides of it that libgit2 compared, each a [`DiffFileRef`]. Everything is
/// lent for the call only, so the closure receives borrows that cannot outlive the call.
///
/// One path can come more than once. Where a directory stands in place of a file the
/// checkout keeps, libgit2 notifies the file as dirty, without a working-directory side,
/// and under `GIT_CHECKOUT_FORCE` as updated, with the directory as that side; it notifies
/// the directory itself, as `<path>/`, as untracked twice.
///
/// The closure returns [`ControlFlow::Continue`] to go on and [`ControlFlow::Break`] to
/// cancel, which the trampoline reports as `GIT_EUSER`: the checkout stops before
/// writing anything and fails with that code. A panic in the closure does not cross into
/// C: the trampoline cancels with `GIT_EUSER`, every later call it gets cancels again
/// without running the closure, and the panic resumes once control is back in Rust, from
/// [`call`](Self::call) or from the wrapper that ran the checkout.
///
/// ```
/// use std::ffi::CStr;
/// use std::ops::ControlFlow;
///
/// use libgit2::ffi;
/// use libgit2::libgit2::src::libgit2::checkout::{CheckoutNotification, NotifyCallback};
///
/// let mut dirty = Vec::new();
/// let mut refuse_dirty = |n: CheckoutNotification<'_>| {
///     if n.why == ffi::GIT_CHECKOUT_NOTIFY_DIRTY {
///         dirty.extend(n.path.map(CStr::to_owned));
///         ControlFlow::Break(())
///     } else {
///         ControlFlow::Continue(())
///     }
/// };
/// let mut callback = NotifyCallback::new(&mut refuse_dirty);
/// let notification = |why, path| CheckoutNotification {
///     why,
///     path: Some(path),
///     baseline: None,
///     target: None,
///     workdir: None,
/// };
/// let updated = notification(ffi::GIT_CHECKOUT_NOTIFY_UPDATED, c"a.txt");
/// assert_eq!(callback.call(updated), ControlFlow::Continue(()));
/// let dirty_file = notification(ffi::GIT_CHECKOUT_NOTIFY_DIRTY, c"b.txt");
/// assert_eq!(callback.call(dirty_file), ControlFlow::Break(()));
/// drop(callback);
/// assert_eq!(dirty, [c"b.txt".to_owned()]);
/// ```
///
/// Wraps `ty::git_checkout_notify_cb`
#[doc(alias = "git_checkout_notify_cb")]
pub struct NotifyCallback<'f> {
    f: &'f mut NotifyFn<'f>,
    /// A panic the closure raised inside C, resumed once control is back in Rust.
    panic: Option<Panic>,
    /// Whether the closure is running: a call that comes meanwhile, from a checkout the
    /// closure started with the options that store this callback, does not run it again.
    busy: bool,
}

impl<'f> NotifyCallback<'f> {
    /// The callback that calls `f` with each notification, until it breaks.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(CheckoutNotification<'_>) -> ControlFlow<()> + 'f,
    {
        Self {
            f,
            panic: None,
            busy: false,
        }
    }

    /// Calls the callback with `notification`, through the trampoline libgit2 calls, and
    /// returns what libgit2 would read from it: [`ControlFlow::Break`] for a cancel. A panic
    /// of the closure resumes here.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic but by resuming the closure's panic: the trampoline is never NULL"
    )]
    pub fn call(&mut self, notification: CheckoutNotification<'_>) -> ControlFlow<()> {
        let CheckoutNotification {
            why,
            path,
            baseline,
            target,
            workdir,
        } = notification;
        let file = |f: Option<DiffFileRef<'_>>| f.map_or(ptr::null(), |f| f.as_ptr());
        let rc = self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            let path = path.map_or(ptr::null(), CStr::as_ptr);
            // SAFETY: `payload` is this callback, which nothing else uses during the call;
            // `path` NULL or a NUL-terminated string, and each file NULL or a live diff file,
            // all borrowed for the call, which only reads them: the trampoline's contract.
            unsafe {
                callback(
                    why,
                    path,
                    file(baseline),
                    file(target),
                    file(workdir),
                    payload,
                )
            }
        });
        if rc == 0 {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    }

    /// Runs `walk` with the C callback and its payload, as [`call`](Self::call) passes them
    /// to the trampoline, or C interop stores them for a call that cannot go through the
    /// options' [setter](CheckoutOptionsMut), then resumes a panic the closure raised
    /// meanwhile. The payload is this callback, which nothing else touches until `walk`
    /// returns; libgit2 must be done calling it by then.
    ///
    /// A walk that unwinds itself, rather than returning, leaves a panic of the closure
    /// behind; it belonged to that walk, so it is dropped here, before the walk it would
    /// otherwise cancel and resume from.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_checkout_notify_cb, *mut c_void) -> R,
    ) -> R {
        self.panic = None;
        let result = walk(Some(notify_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            panic::resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for NotifyCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotifyCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_checkout_notify_cb` over a [`NotifyCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`NotifyCallback`] that only this trampoline's calls use
/// meanwhile, on this thread, `path` be NULL or a NUL-terminated string, and `baseline`,
/// `target` and `workdir` each NULL or a `git_diff_file` whose path is NULL or such a
/// string, all live and unchanged for the call.
#[inline(never)]
unsafe extern "C" fn notify_with(
    why: ffi::git_checkout_notify_t,
    path: *const c_char,
    baseline: *const ffi::git_diff_file,
    target: *const ffi::git_diff_file,
    workdir: *const ffi::git_diff_file,
    payload: *mut c_void,
) -> c_int {
    let callback = payload.cast::<NotifyCallback<'_>>();
    // SAFETY: per this function's contract, `payload` addresses a live callback; the
    // projections form no reference to it, so a nested call, from its closure, aliases none.
    let busy = unsafe { &raw mut (*callback).busy };
    // SAFETY: as above.
    let panic = unsafe { &raw mut (*callback).panic };
    // SAFETY: a read of the live callback's field, on its thread; no reference to it is live.
    let busy_now = unsafe { busy.read() };
    // SAFETY: as above; the reference `is_some` makes ends with the statement.
    let panicked = unsafe { (*panic).is_some() };
    if busy_now || panicked {
        return ErrorCode::User.raw();
    }
    // SAFETY: per this function's contract, a non-NULL `path` is a NUL-terminated string
    // live and unchanged for the call; the borrow ends with it, since the closure is
    // generic over the string's lifetime.
    let path = (!path.is_null()).then(|| unsafe { CStr::from_ptr(path) });
    // SAFETY: per this function's contract, each file is NULL, read as `None`, or live and
    // unchanged for the call; the closure only reads it, through a handle that cannot
    // outlive the call, since the closure is generic over its lifetime.
    let file = |f: *const ffi::git_diff_file| unsafe { DiffFileRef::from_ptr(f.cast_mut()) };
    let notification = CheckoutNotification {
        why,
        path,
        baseline: file(baseline),
        target: file(target),
        workdir: file(workdir),
    };
    // SAFETY: the closure the live callback borrows. Until `busy` is cleared, no other call
    // of this trampoline reaches it, and nothing else does while the callback is lent to
    // C: the options or `walk_with` borrow it exclusively.
    let closure = unsafe { &mut *(*callback).f };
    // SAFETY: a write to the live callback's field; no reference to it is live.
    unsafe { busy.write(true) };
    let result = panic::catch_unwind(AssertUnwindSafe(|| closure(notification)));
    // SAFETY: as above.
    unsafe { busy.write(false) };
    match result {
        Ok(ControlFlow::Continue(())) => 0,
        Ok(ControlFlow::Break(())) => ErrorCode::User.raw(),
        Err(payload) => {
            // SAFETY: as above; the slot is `None`, as checked, and nothing ran the
            // closure meanwhile to fill it.
            unsafe { *panic = Some(payload) };
            ErrorCode::User.raw()
        }
    }
}

/// The closure behind a [`PerfdataCallback`].
type PerfdataFn<'f> = dyn FnMut(CheckoutPerfdataRef<'_>) + 'f;

/// A `git_checkout_perfdata_cb`: the callback a checkout calls once it has written
/// everything, with the filesystem calls it made, as a [`CheckoutPerfdataRef`].
///
/// Built over a closure with [`new`](Self::new). libgit2 calls it synchronously, on the
/// thread running the checkout, exactly once per checkout that gets through its writes,
/// before it writes the index, lending it the counters it kept in its own state for the
/// call alone: the closure receives them as a handle that cannot outlive the call, and
/// keeps a [`copied`](CheckoutPerfdataRef::copied) value. A dry run, and a checkout that
/// fails or is cancelled before then, never call it.
///
/// The callback returns nothing, so it cannot fail the checkout. A panic in the closure
/// does not cross into C: the checkout goes on, and the panic resumes once control is
/// back in Rust, from [`call`](Self::call) or from the wrapper that ran the checkout.
///
/// ```
/// use libgit2::libgit2::src::libgit2::checkout::{
///     CheckoutPerfdata, CheckoutPerfdataRef, PerfdataCallback,
/// };
///
/// let mut seen = Vec::new();
/// let mut record = |perfdata: CheckoutPerfdataRef<'_>| seen.push(perfdata.copied());
/// let mut callback = PerfdataCallback::new(&mut record);
/// let mut counts = CheckoutPerfdata::default();
/// counts.as_mut().set_mkdir_calls(2);
/// counts.as_mut().set_stat_calls(5);
/// callback.call(counts.as_ref());
/// drop(callback);
/// assert_eq!(seen, [counts]);
/// ```
///
/// Wraps `ty::git_checkout_perfdata_cb`
#[doc(alias = "git_checkout_perfdata_cb")]
pub struct PerfdataCallback<'f> {
    f: &'f mut PerfdataFn<'f>,
    /// A panic the closure raised inside C, resumed once control is back in Rust.
    panic: Option<Panic>,
    /// Whether the closure is running: a call that comes meanwhile, from a checkout the
    /// closure started with the options that store this callback, does not run it again.
    busy: bool,
}

impl<'f> PerfdataCallback<'f> {
    /// The callback that calls `f` with each checkout's counters.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(CheckoutPerfdataRef<'_>) + 'f,
    {
        Self {
            f,
            panic: None,
            busy: false,
        }
    }

    /// Calls the callback with `perfdata`, through the trampoline libgit2 calls. A panic
    /// of the closure resumes here.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic but by resuming the closure's panic: the trampoline is never NULL"
    )]
    pub fn call(&mut self, perfdata: CheckoutPerfdataRef<'_>) {
        self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the call;
            // `perfdata` live counters, lent for the call, which only reads them: the
            // trampoline's contract.
            unsafe { callback(perfdata.as_ptr(), payload) }
        });
    }

    /// Runs `walk` with the C callback and its payload, as [`call`](Self::call) passes them
    /// to the trampoline, or C interop stores them for a call that cannot go through the
    /// options' [setter](CheckoutOptionsMut), then resumes a panic the closure raised
    /// meanwhile. The payload is this callback, which nothing else touches until `walk`
    /// returns; libgit2 must be done calling it by then.
    ///
    /// A walk that unwinds itself, rather than returning, leaves a panic of the closure
    /// behind; it belonged to that walk, so it is dropped here, before the walk it would
    /// otherwise skip the closure in and resume from.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_checkout_perfdata_cb, *mut c_void) -> R,
    ) -> R {
        self.panic = None;
        let result = walk(Some(perfdata_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            panic::resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for PerfdataCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PerfdataCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_checkout_perfdata_cb` over a [`PerfdataCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`PerfdataCallback`] that only this trampoline's calls use
/// meanwhile, on this thread, and `perfdata` be NULL or a `git_checkout_perfdata` live and
/// unchanged for the call.
#[inline(never)]
unsafe extern "C" fn perfdata_with(
    perfdata: *const ffi::git_checkout_perfdata,
    payload: *mut c_void,
) {
    let callback = payload.cast::<PerfdataCallback<'_>>();
    // SAFETY: per this function's contract, `payload` addresses a live callback; the
    // projections form no reference to it, so a nested call, from its closure, aliases none.
    let busy = unsafe { &raw mut (*callback).busy };
    // SAFETY: as above.
    let panic = unsafe { &raw mut (*callback).panic };
    // SAFETY: a read of the live callback's field, on its thread; no reference to it is live.
    let busy_now = unsafe { busy.read() };
    // SAFETY: as above; the reference `is_some` makes ends with the statement.
    let panicked = unsafe { (*panic).is_some() };
    if busy_now || panicked {
        return;
    }
    // SAFETY: per this function's contract, a non-NULL `perfdata` is live and unchanged for
    // the call; the closure only reads it, through a handle that cannot outlive the call.
    let Some(perfdata) = (unsafe { CheckoutPerfdataRef::from_ptr(perfdata.cast_mut()) }) else {
        // libgit2 always lends its counters; skip the closure rather than invent them.
        return;
    };
    // SAFETY: the closure the live callback borrows. Until `busy` is cleared, no other call
    // of this trampoline reaches it, and nothing else does while the callback is lent to
    // C: the options or `walk_with` borrow it exclusively.
    let closure = unsafe { &mut *(*callback).f };
    // SAFETY: a write to the live callback's field; no reference to it is live.
    unsafe { busy.write(true) };
    let result = panic::catch_unwind(AssertUnwindSafe(|| closure(perfdata)));
    // SAFETY: as above.
    unsafe { busy.write(false) };
    if let Err(payload) = result {
        // SAFETY: as above; the slot is `None`, as checked, and nothing ran the closure
        // meanwhile to fill it.
        unsafe { *panic = Some(payload) };
    }
}

#[cfg(test)]
mod unit_tests {
    use std::cell::{Cell, RefCell};
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

    /// Every field, as the getters read it, and the callbacks' payloads, as stored.
    fn fields(o: CheckoutOptionsRef<'_, '_>) -> String {
        let (_, notify_payload) = o.notify_fields();
        let (_, progress_payload) = o.progress_fields();
        let (_, perfdata_payload) = o.perfdata_fields();
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?}",
            o,
            notify_payload,
            progress_payload,
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
        assert_eq!(o.notify_callback(), CallbackSlot::Empty);
        assert_eq!(o.progress_callback(), CallbackSlot::Empty);
        assert_eq!(o.perfdata_callback(), CallbackSlot::Empty);
        assert!(o.notify_fields().1.is_null() && o.progress_fields().1.is_null());
        assert!(o.perfdata_fields().1.is_null());
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

    /// Stores C callbacks with `payloads` through the options' raw seam, as C interop does.
    fn store_c_callbacks(m: &mut CheckoutOptionsMut<'_, '_>, payloads: [*mut c_void; 3]) {
        let p = m.as_mut_ptr();
        // SAFETY: the exclusive handle's pointer addresses live options, read whole: plain
        // data, integers and pointers.
        let mut raw = unsafe { p.read() };
        raw.notify_cb = Some(notify);
        raw.progress_cb = Some(progress);
        raw.perfdata_cb = Some(perfdata);
        [
            raw.notify_payload,
            raw.progress_payload,
            raw.perfdata_payload,
        ] = payloads;
        // SAFETY: as above, written back whole; the callbacks ignore their arguments.
        unsafe { p.write(raw) };
    }

    /// What libgit2 does with the options: calls each callback they store, with its
    /// payload, and the arguments `why`, `path` and `perfdata` (no diff files); returns the
    /// notification callback's code, or 0 without one.
    ///
    /// # Safety
    ///
    /// `o` must address live options whose callbacks are sound for such arguments.
    unsafe fn run_callbacks(
        o: *const ffi::git_checkout_options,
        why: u32,
        path: &CStr,
        perfdata: &CheckoutPerfdata,
    ) -> c_int {
        // SAFETY: per this function's contract, `o` addresses live options, read whole.
        let raw = unsafe { o.read() };
        let mut rc = 0;
        if let Some(cb) = raw.notify_cb {
            let null = ptr::null();
            // SAFETY: per this function's contract; the path is NUL-terminated and live.
            rc = unsafe { cb(why, path.as_ptr(), null, null, null, raw.notify_payload) };
        }
        if let Some(cb) = raw.progress_cb {
            // SAFETY: as above.
            unsafe { cb(path.as_ptr(), 1, 2, raw.progress_payload) };
        }
        if let Some(cb) = raw.perfdata_cb {
            let counts = ptr::from_ref(perfdata).cast::<ffi::git_checkout_perfdata>();
            // SAFETY: as above; the counts are live and only read.
            unsafe { cb(counts, raw.perfdata_payload) };
        }
        rc
    }

    #[test]
    #[expect(
        clippy::many_single_char_names,
        reason = "short names for the payloads and handles the steps compare"
    )]
    fn slots_tell_rust_callbacks_from_foreign_ones() {
        let mut payloads = [0u8; 3];
        let [a, b, c] = payloads
            .each_mut()
            .map(|p| ptr::from_mut(p).cast::<c_void>());
        let mut options = template();
        store_c_callbacks(&mut options.as_mut(), [a, b, c]);
        let o = options.as_ref();
        let slots = (
            o.notify_callback(),
            o.progress_callback(),
            o.perfdata_callback(),
        );
        let foreign = CallbackSlot::Foreign;
        assert_eq!(slots, (foreign, foreign, foreign));
        assert_eq!((o.notify_fields().1, o.progress_fields().1), (a, b));
        assert_eq!(o.perfdata_fields().1, c);
        assert!(format!("{o:?}").contains("notify_callback: Foreign"));

        // A payload without a callback is ignored, as libgit2 ignores it.
        options.0.notify_cb = None;
        assert_eq!(options.as_ref().notify_callback(), CallbackSlot::Empty);
        // A trampoline without its payload is not taken for a Rust callback.
        options.0.progress_cb = Some(progress_with);
        options.0.progress_payload = ptr::null_mut();
        assert_eq!(options.as_ref().progress_callback(), CallbackSlot::Foreign);

        let (mut on_note, mut on_progress, mut on_perfdata) = (
            |_: CheckoutNotification<'_>| ControlFlow::Continue(()),
            |_: Option<&CStr>, _, _| {},
            |_: CheckoutPerfdataRef<'_>| {},
        );
        let mut notify = NotifyCallback::new(&mut on_note);
        let mut progress = ProgressCallback::new(&mut on_progress);
        let mut perfdata = PerfdataCallback::new(&mut on_perfdata);
        let expected = (
            ptr::from_mut(&mut notify).cast::<c_void>(),
            ptr::from_mut(&mut progress).cast::<c_void>(),
            ptr::from_mut(&mut perfdata).cast::<c_void>(),
        );
        let mut m = options.as_mut();
        m.set_notify_callback(Some(&mut notify));
        m.set_progress_callback(Some(&mut progress));
        m.set_perfdata_callback(Some(&mut perfdata));
        let o = m.as_ref();
        let rust = CallbackSlot::Rust;
        let slots = (
            o.notify_callback(),
            o.progress_callback(),
            o.perfdata_callback(),
        );
        assert_eq!(slots, (rust, rust, rust));
        let stored = (
            o.notify_fields().1,
            o.progress_fields().1,
            o.perfdata_fields().1,
        );
        assert_eq!(stored, expected);
        assert!(format!("{o:?}").contains("perfdata_callback: Rust"));

        let mut m = options.as_mut();
        m.set_notify_callback(None);
        m.set_progress_callback(None);
        m.set_perfdata_callback(None);
        m.set_baseline(None);
        m.set_baseline_index(None);
        assert_eq!(fields(options.as_ref()), fields(template().as_ref()));
    }

    #[test]
    fn stored_callbacks_run_and_walk_with_resumes_their_panic() {
        let mut notes = Vec::new();
        let mut reports = Vec::new();
        let mut counts = Vec::new();
        let mut on_note = |n: CheckoutNotification<'_>| {
            notes.push(n.path.map(CStr::to_owned));
            ControlFlow::Break(())
        };
        let mut on_progress = |path: Option<&CStr>, done, total| {
            reports.push((path.map(CStr::to_owned), done, total));
            assert!(path != Some(c"b"), "progress closure panics");
        };
        let mut on_perfdata = |d: CheckoutPerfdataRef<'_>| counts.push(d.copied());
        let mut notify = NotifyCallback::new(&mut on_note);
        let mut progress = ProgressCallback::new(&mut on_progress);
        let mut perfdata = PerfdataCallback::new(&mut on_perfdata);
        let mut data = CheckoutPerfdata::default();
        data.as_mut().set_stat_calls(3);
        {
            let mut options = template();
            let mut m = options.as_mut();
            m.set_notify_callback(Some(&mut notify));
            m.set_progress_callback(Some(&mut progress));
            m.set_perfdata_callback(Some(&mut perfdata));
            let o = options.as_ref();
            // SAFETY: the options are live, and their trampolines sound for these arguments.
            let rc = o.walk_with(|o| unsafe { run_callbacks(o, 1, c"a", &data) });
            assert_eq!(rc, ErrorCode::User.raw());
            // The progress closure panics on `b`; the panic resumes from the walk, after
            // every callback ran.
            let walk = || {
                // SAFETY: as above.
                o.walk_with(|o| unsafe { run_callbacks(o, 1, c"b", &data) });
            };
            let walk = panic::catch_unwind(AssertUnwindSafe(walk));
            let panic = walk.expect_err("the progress panic resumes");
            assert_eq!(
                panic.downcast_ref::<&str>(),
                Some(&"progress closure panics")
            );
            // Nothing is left to resume.
            // SAFETY: as above.
            let rc = o.walk_with(|o| unsafe { run_callbacks(o, 1, c"c", &data) });
            assert_eq!(rc, ErrorCode::User.raw());
        }
        drop((notify, perfdata));
        drop(progress);
        let c = |s: &CStr| Some(s.to_owned());
        assert_eq!(notes, [c(c"a"), c(c"b"), c(c"c")]);
        assert_eq!(reports.len(), 3);
        assert_eq!(counts, [data; 3]);
    }

    #[test]
    fn walk_with_resumes_the_notify_panic_first_and_drops_the_rest() {
        let mut on_note = |_: CheckoutNotification<'_>| -> ControlFlow<()> { panic!("notify") };
        let mut on_progress = |_: Option<&CStr>, _, _| panic!("progress");
        let mut on_perfdata = |_: CheckoutPerfdataRef<'_>| panic!("perfdata");
        let mut notify = NotifyCallback::new(&mut on_note);
        let mut progress = ProgressCallback::new(&mut on_progress);
        let mut perfdata = PerfdataCallback::new(&mut on_perfdata);
        let data = CheckoutPerfdata::default();
        let mut options = template();
        let mut m = options.as_mut();
        m.set_notify_callback(Some(&mut notify));
        m.set_progress_callback(Some(&mut progress));
        m.set_perfdata_callback(Some(&mut perfdata));
        let o = options.as_ref();
        for _ in 0..2 {
            let walk = || {
                // SAFETY: the options are live, and their trampolines sound for these
                // arguments.
                o.walk_with(|o| unsafe { run_callbacks(o, 1, c"a", &data) })
            };
            let panic = panic::catch_unwind(AssertUnwindSafe(walk)).unwrap_err();
            assert_eq!(panic.downcast_ref::<&str>(), Some(&"notify"));
        }
        // Each slot was emptied: a walk that calls nothing resumes nothing.
        assert_eq!(o.walk_with(|_| 7), 7);
    }

    #[test]
    fn walk_with_drops_a_stale_panic_and_leaves_foreign_callbacks_alone() {
        let mut calls = 0;
        let mut on_progress = |_: Option<&CStr>, _, _| {
            calls += 1;
            panic!("stale");
        };
        let mut progress = ProgressCallback::new(&mut on_progress);
        let data = CheckoutPerfdata::default();
        let mut options = template();
        // Foreign callbacks whose payloads must never be read: dangling pointers.
        let dangling = ptr::dangling_mut::<u64>().cast::<c_void>();
        store_c_callbacks(&mut options.as_mut(), [dangling; 3]);
        options.as_mut().set_progress_callback(Some(&mut progress));
        // Handed to C directly, without a walk: the panic stays in the callback.
        // SAFETY: the options are live, and their callbacks sound for these arguments.
        let rc = unsafe { run_callbacks(options.as_ref().as_ptr(), 1, c"a", &data) };
        assert_eq!(rc, 0);
        // The next walk drops it before C runs: the closure runs again, and its new panic
        // is the one resumed.
        let walk = || {
            // SAFETY: as above.
            options
                .as_ref()
                .walk_with(|o| unsafe { run_callbacks(o, 1, c"b", &data) })
        };
        assert!(panic::catch_unwind(AssertUnwindSafe(walk)).is_err());
        assert_eq!(options.as_ref().notify_callback(), CallbackSlot::Foreign);
        // Storing the callback again also drops what it holds.
        // SAFETY: as above.
        unsafe { run_callbacks(options.as_ref().as_ptr(), 1, c"c", &data) };
        let mut options = template();
        options.as_mut().set_progress_callback(Some(&mut progress));
        let walk = || {
            // SAFETY: as above.
            options
                .as_ref()
                .walk_with(|o| unsafe { run_callbacks(o, 1, c"d", &data) })
        };
        assert!(panic::catch_unwind(AssertUnwindSafe(walk)).is_err());
        drop(progress);
        assert_eq!(calls, 4);
    }

    #[test]
    fn a_call_from_inside_the_closure_finds_the_callback_busy() {
        let options = Cell::new(ptr::null::<ffi::git_checkout_options>());
        let data = CheckoutPerfdata::default();
        let depth = Cell::new(0);
        let events = RefCell::new(Vec::new());
        let nested = RefCell::new(Vec::new());
        // Each closure logs its call and, at the outer level, runs the options' callbacks
        // again, as a checkout it started with them would.
        let run = |name: char| {
            events.borrow_mut().push((name, depth.get()));
            if depth.get() == 0 {
                depth.set(1);
                // SAFETY: the options are live and their trampolines sound for these
                // arguments; this is the nested call a closure can make.
                let rc = unsafe { run_callbacks(options.get(), 2, c"inner", &data) };
                depth.set(0);
                nested.borrow_mut().push(rc);
            }
        };
        let mut on_note = |_: CheckoutNotification<'_>| {
            run('n');
            ControlFlow::Continue(())
        };
        let mut on_progress = |_: Option<&CStr>, _, _| run('p');
        let mut on_perfdata = |_: CheckoutPerfdataRef<'_>| run('f');
        let mut notify = NotifyCallback::new(&mut on_note);
        let mut progress = ProgressCallback::new(&mut on_progress);
        let mut perfdata = PerfdataCallback::new(&mut on_perfdata);
        let mut o = template();
        let mut m = o.as_mut();
        m.set_notify_callback(Some(&mut notify));
        m.set_progress_callback(Some(&mut progress));
        m.set_perfdata_callback(Some(&mut perfdata));
        options.set(o.as_ref().as_ptr());
        // SAFETY: as above.
        let rc = o
            .as_ref()
            .walk_with(|o| unsafe { run_callbacks(o, 1, c"outer", &data) });
        assert_eq!(rc, 0);
        // Inside each closure, its own callback is busy (a nested notification is
        // cancelled, a nested report skipped) and the other two run.
        let events = events.borrow().clone();
        let expected = [
            ('n', 0),
            ('p', 1),
            ('f', 1),
            ('p', 0),
            ('n', 1),
            ('f', 1),
            ('f', 0),
            ('n', 1),
            ('p', 1),
        ];
        assert_eq!(events, expected);
        assert_eq!(nested.borrow().clone(), [ErrorCode::User.raw(), 0, 0]);
    }

    #[test]
    fn a_nested_walk_leaves_the_outer_walks_panics_to_it() {
        let options = Cell::new(ptr::null::<ffi::git_checkout_options>());
        let data = CheckoutPerfdata::default();
        let events = RefCell::new(Vec::new());
        let nested = Cell::new(None);
        // The progress closure panics at the outer level; the performance-data closure then
        // checks out again with the same options, through their walk, as a closure calling
        // `Worktree::add` with its own options does.
        let mut on_progress = |path: Option<&CStr>, _, _| {
            events.borrow_mut().push(('p', path.map(CStr::to_owned)));
            assert!(path != Some(c"outer"), "progress");
        };
        let mut on_perfdata = |_: CheckoutPerfdataRef<'_>| {
            events.borrow_mut().push(('f', None));
            if nested.get().is_none() {
                // SAFETY: the options are live for the outer walk, which this closure runs
                // in, and only read through the shared handle.
                let o = unsafe { CheckoutOptionsRef::from_ptr(options.get().cast_mut()) };
                let walk = || {
                    // SAFETY: the options are live, and their trampolines sound for these
                    // arguments.
                    o.unwrap()
                        .walk_with(|o| unsafe { run_callbacks(o, 1, c"inner", &data) })
                };
                nested.set(Some(panic::catch_unwind(AssertUnwindSafe(walk)).is_ok()));
            }
        };
        let mut progress = ProgressCallback::new(&mut on_progress);
        let mut perfdata = PerfdataCallback::new(&mut on_perfdata);
        let mut o = template();
        let mut m = o.as_mut();
        m.set_progress_callback(Some(&mut progress));
        m.set_perfdata_callback(Some(&mut perfdata));
        options.set(o.as_ref().as_ptr());
        let walk = || {
            // SAFETY: as above.
            o.as_ref()
                .walk_with(|o| unsafe { run_callbacks(o, 1, c"outer", &data) })
        };
        let panic = panic::catch_unwind(AssertUnwindSafe(walk)).unwrap_err();
        // The outer walk resumes its own panic; the nested walk resumed nothing, and skipped
        // the panicked progress closure, as the rest of the outer walk does.
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"progress"));
        assert_eq!(nested.get(), Some(true));
        let seen = events.borrow().clone();
        assert_eq!(seen, [('p', Some(c"outer".to_owned())), ('f', None)]);
        // The slot was emptied: the next walk runs the closure again.
        // SAFETY: as above.
        let rc = o
            .as_ref()
            .walk_with(|o| unsafe { run_callbacks(o, 1, c"again", &data) });
        assert_eq!(rc, 0);
        assert_eq!(events.borrow().len(), 4);
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
            "notify_callback: Empty",
            "progress_callback: Empty",
            "paths: [\"x\"]",
            "baseline: false",
            "baseline_index: false",
            "target_directory: None",
            "ancestor_label: None",
            "our_label: Some(\"mine\")",
            "their_label: None",
            "perfdata_callback: Empty",
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

    /// The notification and performance-data callbacks' tests, over the progress tests'
    /// fixture.
    mod notify_and_perfdata_callbacks {
        use std::fs;
        use std::path::Path;

        use super::progress_callback::{FILES, FORCE, Staged};
        use super::*;

        const DRY_RUN: u32 = 1 << 24;
        const UPDATED: u32 = ffi::GIT_CHECKOUT_NOTIFY_UPDATED;
        const UNTRACKED: u32 = ffi::GIT_CHECKOUT_NOTIFY_UNTRACKED;
        const CONFLICT: u32 = ffi::GIT_CHECKOUT_NOTIFY_CONFLICT;
        const DIRTY: u32 = ffi::GIT_CHECKOUT_NOTIFY_DIRTY;
        const IGNORED: u32 = ffi::GIT_CHECKOUT_NOTIFY_IGNORED;
        const USER: c_int = ffi::GIT_EUSER;

        /// One side of a notification, as owned data: its path, mode and size.
        type Side = Option<(Option<CString>, u16, u64)>;

        /// One notification: the reason, the path and the baseline, target and workdir
        /// sides.
        type Note = (u32, Option<CString>, Side, Side, Side);

        fn side(file: Option<DiffFileRef<'_>>) -> Side {
            file.map(|f| (f.path().map(CStr::to_owned), f.mode(), f.size()))
        }

        fn note(n: CheckoutNotification<'_>) -> Note {
            let path = n.path.map(CStr::to_owned);
            (
                n.why,
                path,
                side(n.baseline),
                side(n.target),
                side(n.workdir),
            )
        }

        /// A notification without sides.
        fn bare(why: u32, path: Option<&CStr>) -> CheckoutNotification<'_> {
            CheckoutNotification {
                why,
                path,
                baseline: None,
                target: None,
                workdir: None,
            }
        }

        /// Fills `dir` as a stale working directory: `a.txt` changed, `dir/b.txt` as
        /// staged, `dir/sub/c.txt` missing and an untracked `extra.txt`.
        fn dirty(dir: &Path) {
            fs::create_dir_all(dir.join("dir")).unwrap();
            fs::write(dir.join("a.txt"), "changed\n").unwrap();
            fs::write(dir.join("dir/b.txt"), "two\n").unwrap();
            fs::write(dir.join("extra.txt"), "x\n").unwrap();
        }

        /// Checks the index out into `target` with `strategy`, notifying on everything,
        /// with the callbacks stored in the options, and returns libgit2's code.
        fn checkout(
            staged: &Staged,
            target: &CStr,
            strategy: u32,
            notify: &mut NotifyCallback<'_>,
            perfdata: &mut PerfdataCallback<'_>,
        ) -> c_int {
            checkout_against(staged, target, strategy, None, notify, perfdata)
        }

        /// [`checkout`], against `baseline` rather than `HEAD` when it is given.
        fn checkout_against(
            staged: &Staged,
            target: &CStr,
            strategy: u32,
            baseline: Option<IndexRef<'_>>,
            notify: &mut NotifyCallback<'_>,
            perfdata: &mut PerfdataCallback<'_>,
        ) -> c_int {
            let mut options = CheckoutOptions::new();
            options.as_mut().set_baseline_index(baseline);
            options.as_mut().set_strategy(strategy);
            options
                .as_mut()
                .set_notify_flags(ffi::GIT_CHECKOUT_NOTIFY_ALL);
            options.as_mut().set_target_directory(Some(target));
            options.as_mut().set_notify_callback(Some(notify));
            options.as_mut().set_perfdata_callback(Some(perfdata));
            options.as_ref().walk_with(|o| {
                // SAFETY: the live repository and index; live options, whose pointers are
                // live or NULL and whose callbacks the walk resumes, read for the call.
                unsafe { ffi::git_checkout_index(staged.repo, staged.index, o) }
            })
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn libgit2_notifies_every_path_with_its_sides_then_reports_its_counts() {
            let staged = Staged::new("notify-report", FILES);
            let (path, target) = staged.target("out");
            dirty(&path);
            let mut notes = Vec::new();
            let mut record = |n: CheckoutNotification<'_>| {
                notes.push(note(n));
                ControlFlow::Continue(())
            };
            let mut counts = Vec::new();
            let mut count = |d: CheckoutPerfdataRef<'_>| counts.push(d.copied());
            let mut notify = NotifyCallback::new(&mut record);
            let mut perfdata = PerfdataCallback::new(&mut count);
            let rc = checkout(&staged, &target, FORCE, &mut notify, &mut perfdata);
            drop((notify, perfdata));
            assert_eq!(rc, 0);
            let file = |p: &CStr, size| Some((Some(p.to_owned()), 0o100_644, size));
            assert_eq!(
                notes,
                [
                    (
                        UPDATED,
                        Some(c"a.txt".to_owned()),
                        None,
                        file(c"a.txt", 4),
                        file(c"a.txt", 8)
                    ),
                    (
                        UPDATED,
                        Some(c"dir/b.txt".to_owned()),
                        None,
                        file(c"dir/b.txt", 4),
                        file(c"dir/b.txt", 4)
                    ),
                    (
                        UPDATED,
                        Some(c"dir/sub/c.txt".to_owned()),
                        None,
                        file(c"dir/sub/c.txt", 6),
                        None
                    ),
                    (
                        UNTRACKED,
                        Some(c"extra.txt".to_owned()),
                        None,
                        None,
                        file(c"extra.txt", 2)
                    ),
                ]
            );
            // Once, after the writes: `dir/sub` had to be made.
            assert_eq!(counts.len(), 1);
            assert!(counts[0].as_ref().mkdir_calls() >= 1, "{counts:?}");
            assert!(counts[0].as_ref().stat_calls() >= 1, "{counts:?}");
            assert_eq!(fs::read(path.join("a.txt")).unwrap(), b"one\n");
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_break_cancels_before_any_write_and_skips_the_counts() {
            let staged = Staged::new("notify-break", FILES);
            let (path, target) = staged.target("out");
            dirty(&path);
            let mut calls = 0;
            let mut refuse = |n: CheckoutNotification<'_>| {
                calls += 1;
                if n.why == UNTRACKED {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let mut counted = 0;
            let mut count = |_: CheckoutPerfdataRef<'_>| counted += 1;
            let mut notify = NotifyCallback::new(&mut refuse);
            let mut perfdata = PerfdataCallback::new(&mut count);
            let rc = checkout(&staged, &target, FORCE, &mut notify, &mut perfdata);
            drop((notify, perfdata));
            assert_eq!((rc, calls, counted), (USER, 4, 0));
            // Nothing was written: the stale file stands, the missing one is still missing.
            assert_eq!(fs::read(path.join("a.txt")).unwrap(), b"changed\n");
            assert!(!path.join("dir/sub/c.txt").exists());
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_safe_checkout_notifies_conflicts_and_a_dry_run_counts_nothing() {
            let staged = Staged::new("notify-conflict", FILES);
            let (path, target) = staged.target("out");
            dirty(&path);
            for (strategy, expected) in [(0, ffi::GIT_ECONFLICT), (FORCE | DRY_RUN, 0)] {
                let mut whys = Vec::new();
                let mut record = |n: CheckoutNotification<'_>| {
                    whys.push(n.why);
                    ControlFlow::Continue(())
                };
                let mut counted = 0;
                let mut count = |_: CheckoutPerfdataRef<'_>| counted += 1;
                let mut notify = NotifyCallback::new(&mut record);
                let mut perfdata = PerfdataCallback::new(&mut count);
                let rc = checkout(&staged, &target, strategy, &mut notify, &mut perfdata);
                drop((notify, perfdata));
                assert_eq!((rc, counted), (expected, 0), "{strategy:#x}");
                if strategy == 0 {
                    assert_eq!(whys, [CONFLICT, CONFLICT, UPDATED, UNTRACKED]);
                } else {
                    assert_eq!(whys, [UPDATED, UPDATED, UPDATED, UNTRACKED]);
                }
            }
            assert_eq!(fs::read(path.join("a.txt")).unwrap(), b"changed\n");
        }

        /// A notification's reason, path and sides, each side as its path and mode: a
        /// directory's size is the filesystem's.
        type Shape = (u32, Option<CString>, [Option<(Option<CString>, u16)>; 3]);

        /// A directory standing in place of a file the baseline and the target share, and
        /// a file the repository ignores, checked out against the index itself, safely and
        /// forced. The file is notified as dirty without a working-directory side, and
        /// when forced as updated too, with the directory as that side; the directory, as
        /// `a.txt/`, is notified as untracked twice; the ignored file as ignored.
        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_directory_in_place_of_a_kept_file_is_notified_more_than_once() {
            let staged = Staged::new("notify-dir", FILES);
            // Ignore rules are the repository's, not the target directory's.
            fs::write(staged.dir.join("work/.git/info/exclude"), "*.log\n").unwrap();
            // SAFETY: the fixture's live index, which outlives every checkout below.
            let index = unsafe { IndexRef::from_ptr(staged.index) };
            let side = |p: &CStr, mode| Some((Some(p.to_owned()), mode));
            let (file, dir) = (side(c"a.txt", 0o100_644), side(c"a.txt/", 0o040_000));
            let a = Some(c"a.txt".to_owned());
            let dirty: Shape = (DIRTY, a.clone(), [file.clone(), file.clone(), None]);
            let updated: Shape = (UPDATED, a, [file.clone(), file, dir.clone()]);
            let untracked: Shape = (UNTRACKED, Some(c"a.txt/".to_owned()), [None, None, dir]);
            let log = side(c"x.log", 0o100_644);
            let ignored: Shape = (IGNORED, Some(c"x.log".to_owned()), [None, None, log]);
            let safe = vec![
                dirty.clone(),
                untracked.clone(),
                untracked.clone(),
                ignored.clone(),
            ];
            let forced = vec![dirty, untracked.clone(), updated, untracked, ignored];
            for (strategy, expected) in [(0, safe), (FORCE, forced)] {
                let (path, target) = staged.target(&format!("out-{strategy}"));
                for (name, content) in FILES {
                    let file = path.join(name);
                    fs::create_dir_all(file.parent().unwrap()).unwrap();
                    fs::write(file, content).unwrap();
                }
                fs::remove_file(path.join("a.txt")).unwrap();
                fs::create_dir(path.join("a.txt")).unwrap();
                fs::write(path.join("a.txt/inner"), "in\n").unwrap();
                fs::write(path.join("x.log"), "log\n").unwrap();
                let mut shapes = Vec::new();
                let mut record = |n: CheckoutNotification<'_>| {
                    let (why, path, sides) = (n.why, n.path.map(CStr::to_owned), n);
                    let side = |f: Option<DiffFileRef<'_>>| {
                        f.map(|f| (f.path().map(CStr::to_owned), f.mode()))
                    };
                    let sides = [
                        side(sides.baseline),
                        side(sides.target),
                        side(sides.workdir),
                    ];
                    shapes.push((why, path, sides));
                    ControlFlow::Continue(())
                };
                let mut ignore = |_: CheckoutPerfdataRef<'_>| {};
                let mut notify = NotifyCallback::new(&mut record);
                let mut perfdata = PerfdataCallback::new(&mut ignore);
                let rc = checkout_against(
                    &staged,
                    &target,
                    strategy,
                    index,
                    &mut notify,
                    &mut perfdata,
                );
                drop((notify, perfdata));
                assert_eq!(rc, 0, "{strategy:#x}");
                assert_eq!(shapes, expected, "{strategy:#x}");
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_notify_panic_cancels_the_checkout_then_resumes() {
            let staged = Staged::new("notify-panic", FILES);
            let (path, target) = staged.target("out");
            let mut calls = 0;
            let mut boom = |n: CheckoutNotification<'_>| {
                calls += 1;
                assert!(n.path != Some(c"dir/b.txt"), "boom");
                ControlFlow::Continue(())
            };
            let mut count = |_: CheckoutPerfdataRef<'_>| unreachable!("the checkout is cancelled");
            let mut notify = NotifyCallback::new(&mut boom);
            let mut perfdata = PerfdataCallback::new(&mut count);
            let caught = panic::catch_unwind(AssertUnwindSafe(|| {
                checkout(&staged, &target, FORCE, &mut notify, &mut perfdata)
            }));
            assert_eq!(*caught.unwrap_err().downcast::<&str>().unwrap(), "boom");
            assert!(format!("{notify:?}").contains("panicked: false"));
            drop((notify, perfdata));
            // The planning stops at the cancelled path: nothing is written.
            assert_eq!(calls, 2);
            assert!(!path.join("a.txt").exists());
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls C")]
        fn a_perfdata_panic_lets_the_checkout_finish_then_resumes() {
            let staged = Staged::new("perfdata-panic", FILES);
            let (path, target) = staged.target("out");
            let mut go_on = |_: CheckoutNotification<'_>| ControlFlow::Continue(());
            let mut boom = |_: CheckoutPerfdataRef<'_>| panic!("boom");
            let mut notify = NotifyCallback::new(&mut go_on);
            let mut perfdata = PerfdataCallback::new(&mut boom);
            let caught = panic::catch_unwind(AssertUnwindSafe(|| {
                checkout(&staged, &target, FORCE, &mut notify, &mut perfdata)
            }));
            assert_eq!(*caught.unwrap_err().downcast::<&str>().unwrap(), "boom");
            assert!(format!("{perfdata:?}").contains("panicked: false"));
            drop((notify, perfdata));
            for (file, content) in FILES {
                assert_eq!(fs::read(path.join(file)).unwrap(), content.as_bytes());
            }
        }

        /// A `git_diff_file` over `path`, as libgit2 lends one.
        fn raw_file(path: &CStr, size: u64) -> ffi::git_diff_file {
            ffi::git_diff_file {
                path: path.as_ptr(),
                size,
                mode: 0o100_644,
                ..Default::default()
            }
        }

        #[test]
        fn call_lends_the_reason_path_and_sides() {
            let mut notes = Vec::new();
            let mut record = |n: CheckoutNotification<'_>| {
                notes.push(note(n));
                ControlFlow::Continue(())
            };
            let mut callback = NotifyCallback::new(&mut record);
            let owned = CString::new("dir/é.txt").unwrap();
            let mut old = raw_file(&owned, 3);
            let mut new = raw_file(c"renamed", u64::MAX);
            // SAFETY: a live diff file whose path outlives the handle, which only reads it.
            let b = unsafe { DiffFileRef::from_ptr(&raw mut old) };
            // SAFETY: as above.
            let t = unsafe { DiffFileRef::from_ptr(&raw mut new) };
            let full = CheckoutNotification {
                why: CONFLICT,
                path: Some(&owned),
                baseline: b,
                target: t,
                workdir: b,
            };
            assert_eq!(
                format!("{full:?}"),
                "CheckoutNotification { why: 0x1, path: Some(\"dir/é.txt\"), \
                 baseline: Some(\"dir/é.txt\"), target: Some(\"renamed\"), \
                 workdir: Some(\"dir/é.txt\") }"
            );
            assert_eq!(callback.call(full), ControlFlow::Continue(()));
            assert_eq!(callback.call(bare(0, None)), ControlFlow::Continue(()));
            assert_eq!(
                format!("{callback:?}"),
                "NotifyCallback { panicked: false, .. }"
            );
            drop(callback);
            let file = |p: &CStr, size| Some((Some(p.to_owned()), 0o100_644, size));
            assert_eq!(
                notes,
                [
                    (
                        CONFLICT,
                        Some(owned.clone()),
                        file(&owned, 3),
                        file(c"renamed", u64::MAX),
                        file(&owned, 3)
                    ),
                    (0, None, None, None, None),
                ]
            );
        }

        #[test]
        fn the_notify_trampoline_reads_nulls_as_none_and_breaks_as_euser() {
            let mut seen = Vec::new();
            let mut record = |n: CheckoutNotification<'_>| {
                let none = (
                    n.baseline.is_none(),
                    n.target.is_none(),
                    n.workdir.is_none(),
                );
                seen.push((n.why, n.path.is_none(), none));
                if n.why == 0 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            };
            let mut callback = NotifyCallback::new(&mut record);
            let mut file = raw_file(c"x", 1);
            let file_ptr = (&raw mut file).cast_const();
            let rcs = callback.walk_with(|cb, payload| {
                let cb = cb.unwrap();
                // SAFETY: `payload` is the callback `walk_with` lends; NULLs are in the
                // trampoline's contract.
                let first = unsafe {
                    cb(
                        0,
                        ptr::null(),
                        ptr::null(),
                        ptr::null(),
                        ptr::null(),
                        payload,
                    )
                };
                // SAFETY: as above, with a path and a file live for the call.
                let second = unsafe {
                    cb(
                        UPDATED,
                        c"x".as_ptr(),
                        ptr::null(),
                        file_ptr,
                        ptr::null(),
                        payload,
                    )
                };
                (first, second)
            });
            drop(callback);
            assert_eq!(rcs, (0, USER));
            assert_eq!(
                seen,
                [
                    (0, true, (true, true, true)),
                    (UPDATED, false, (true, false, true))
                ]
            );
        }

        #[test]
        fn a_notify_panic_cancels_every_later_call_of_the_walk_then_resumes() {
            let mut calls = 0;
            let mut boom = |_: CheckoutNotification<'_>| -> ControlFlow<()> {
                calls += 1;
                panic!("boom");
            };
            let mut callback = NotifyCallback::new(&mut boom);
            let mut rcs = Vec::new();
            let caught = panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    let cb = cb.unwrap();
                    for _ in 0..3 {
                        // SAFETY: `payload` is the callback `walk_with` lends.
                        rcs.push(unsafe {
                            cb(
                                UPDATED,
                                ptr::null(),
                                ptr::null(),
                                ptr::null(),
                                ptr::null(),
                                payload,
                            )
                        });
                    }
                });
            }));
            assert_eq!(*caught.unwrap_err().downcast::<&str>().unwrap(), "boom");
            assert_eq!(rcs, [USER; 3]);
            assert!(format!("{callback:?}").contains("panicked: false"));
            drop(callback);
            assert_eq!(calls, 1);
        }

        #[test]
        fn notify_call_resumes_a_panic_and_the_callback_runs_again() {
            let mut calls = 0;
            let mut f = |n: CheckoutNotification<'_>| {
                calls += 1;
                assert_ne!(n.why, CONFLICT, "conflict");
                ControlFlow::Continue(())
            };
            let mut callback = NotifyCallback::new(&mut f);
            let caught =
                panic::catch_unwind(AssertUnwindSafe(|| callback.call(bare(CONFLICT, None))));
            assert!(caught.is_err());
            assert_eq!(
                callback.call(bare(UPDATED, None)),
                ControlFlow::Continue(())
            );
            drop(callback);
            assert_eq!(calls, 2);
        }

        #[test]
        fn a_notify_walk_that_unwinds_leaves_no_stale_panic() {
            let mut calls = 0;
            let mut f = |_: CheckoutNotification<'_>| {
                calls += 1;
                assert!(calls > 1, "first call");
                ControlFlow::Continue(())
            };
            let mut callback = NotifyCallback::new(&mut f);
            let walked = panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    // SAFETY: `payload` is the callback `walk_with` lends.
                    unsafe {
                        cb.unwrap()(
                            UPDATED,
                            ptr::null(),
                            ptr::null(),
                            ptr::null(),
                            ptr::null(),
                            payload,
                        )
                    };
                    panic!("the walk's own");
                })
            }));
            assert_eq!(
                *walked.unwrap_err().downcast::<&str>().unwrap(),
                "the walk's own"
            );
            assert_eq!(
                callback.call(bare(UPDATED, None)),
                ControlFlow::Continue(())
            );
            drop(callback);
            assert_eq!(calls, 2);
        }

        #[test]
        fn perfdata_call_lends_the_counts() {
            let mut seen = Vec::new();
            let mut record = |d: CheckoutPerfdataRef<'_>| seen.push(d.copied());
            let mut callback = PerfdataCallback::new(&mut record);
            let mut counts = CheckoutPerfdata::default();
            callback.call(counts.as_ref());
            counts.as_mut().set_mkdir_calls(usize::MAX);
            counts.as_mut().set_chmod_calls(3);
            callback.call(counts.as_ref());
            assert_eq!(
                format!("{callback:?}"),
                "PerfdataCallback { panicked: false, .. }"
            );
            drop(callback);
            assert_eq!(seen, [CheckoutPerfdata::default(), counts]);
        }

        #[test]
        fn the_perfdata_trampoline_skips_a_null_and_reads_c_counts() {
            let mut seen = Vec::new();
            let mut record = |d: CheckoutPerfdataRef<'_>| seen.push(d.copied());
            let mut callback = PerfdataCallback::new(&mut record);
            let raw = ffi::git_checkout_perfdata {
                mkdir_calls: 1,
                stat_calls: 2,
                chmod_calls: 3,
            };
            callback.walk_with(|cb, payload| {
                let cb = cb.unwrap();
                // SAFETY: `payload` is the callback `walk_with` lends; NULL is in the
                // trampoline's contract.
                unsafe { cb(ptr::null(), payload) };
                // SAFETY: as above, with counters live for the call.
                unsafe { cb(&raw const raw, payload) };
            });
            drop(callback);
            assert_eq!(seen, [counts(1, 2, 3)]);
        }

        #[test]
        fn a_perfdata_panic_skips_later_calls_of_the_walk_then_resumes() {
            let mut calls = 0;
            let mut boom = |_: CheckoutPerfdataRef<'_>| {
                calls += 1;
                panic!("boom");
            };
            let mut callback = PerfdataCallback::new(&mut boom);
            let counts = CheckoutPerfdata::default();
            let caught = panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    for _ in 0..2 {
                        // SAFETY: `payload` is the callback `walk_with` lends.
                        unsafe { cb.unwrap()(counts.as_ref().as_ptr(), payload) };
                    }
                });
            }));
            assert_eq!(*caught.unwrap_err().downcast::<&str>().unwrap(), "boom");
            // The same callback runs again: the walk's panic was resumed, not kept.
            let caught = panic::catch_unwind(AssertUnwindSafe(|| callback.call(counts.as_ref())));
            assert!(caught.is_err());
            drop(callback);
            assert_eq!(calls, 2);
        }

        #[test]
        fn a_perfdata_walk_that_unwinds_leaves_no_stale_panic() {
            let mut calls = 0;
            let mut f = |_: CheckoutPerfdataRef<'_>| {
                calls += 1;
                assert!(calls > 1, "first call");
            };
            let mut callback = PerfdataCallback::new(&mut f);
            let counts = CheckoutPerfdata::default();
            let walked = panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    // SAFETY: `payload` is the callback `walk_with` lends.
                    unsafe { cb.unwrap()(counts.as_ref().as_ptr(), payload) };
                    panic!("the walk's own");
                })
            }));
            assert_eq!(
                *walked.unwrap_err().downcast::<&str>().unwrap(),
                "the walk's own"
            );
            callback.call(counts.as_ref());
            drop(callback);
            assert_eq!(calls, 2);
        }
    }

    /// The progress callback's tests.
    mod progress_callback {
        use std::path::PathBuf;
        use std::sync::OnceLock;
        use std::{fs, process};

        use super::*;

        /// `GIT_CHECKOUT_FORCE`, from `git2/checkout.h`: the `-sys` crate does not bind the
        /// strategy enum.
        pub(super) const FORCE: u32 = 1 << 1;

        /// What a closure saw: the path (owned) and the two counts of each call.
        type Seen = Vec<(Option<CString>, usize, usize)>;

        /// A repository under a scratch directory, its working directory holding `files`,
        /// all staged in its index; released on drop.
        pub(super) struct Staged {
            pub(super) dir: PathBuf,
            pub(super) repo: *mut ffi::git_repository,
            pub(super) index: *mut ffi::git_index,
        }

        impl Staged {
            pub(super) fn new(tag: &str, files: &[(&str, &str)]) -> Self {
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

            /// Checks the index out into `target`, forced, with the callback stored in the
            /// options, and returns libgit2's code.
            fn checkout(&self, target: &CStr, callback: &mut ProgressCallback<'_>) -> c_int {
                let mut options = CheckoutOptions::new();
                options.as_mut().set_strategy(FORCE);
                options.as_mut().set_target_directory(Some(target));
                options.as_mut().set_progress_callback(Some(callback));
                options.as_ref().walk_with(|o| {
                    // SAFETY: the live repository and index; live options, whose pointers
                    // are live or NULL and whose callback the walk resumes, read for the
                    // call.
                    unsafe { ffi::git_checkout_index(self.repo, self.index, o) }
                })
            }

            pub(super) fn target(&self, name: &str) -> (PathBuf, CString) {
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

        pub(super) const FILES: &[(&str, &str)] = &[
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
            options.as_mut().set_progress_callback(Some(&mut callback));
            let rc = options.as_ref().walk_with(|o| {
                // SAFETY: as in `Staged::checkout`.
                unsafe { ffi::git_checkout_index(staged.repo, staged.index, o) }
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
