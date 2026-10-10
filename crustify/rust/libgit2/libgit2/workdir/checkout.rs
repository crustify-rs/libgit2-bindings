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
//! [`IndexBox`](crate::libgit2::index::index::IndexBox) or
//! [`IndexArc`](crate::libgit2::index::index::IndexArc) holding it. The pointers whose
//! types this crate does not wrap yet stay raw seams, with `unsafe` setters whose
//! callers vouch for what the borrow checker cannot: the baseline tree (`git_tree`),
//! and the notification, progress and performance callbacks with their payloads. The flags fields take the raw bits of
//! `git_checkout_strategy_t` and `git_checkout_notify_t` until those are wrapped.
//!
//! `git_rebase_init` and `git_rebase_open` are the one exception to "read during the
//! call": the rebase keeps a copy of the options, duplicating the strings and the
//! baseline tree but keeping the `paths` array, the callbacks and payloads, and the
//! baseline index as given, and `git_rebase_free` releases that index without having
//! taken a reference to it. Their wrappers must keep what the copy borrows alive for
//! the rebase, and must not hand them a baseline index.

use core::ffi::{CStr, c_int, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ptr::{self, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CCell};

use crate::ffi;
use crate::libgit2::core::strarray::StrArrayRef;
use crate::libgit2::index::index::IndexRef;

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
/// use libgit2::libgit2::workdir::checkout::CheckoutOptions;
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
/// use libgit2::libgit2::workdir::checkout::CheckoutOptionsMut;
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
/// use libgit2::libgit2::workdir::checkout::CheckoutOptions;
///
/// let options = CheckoutOptions::new();
/// std::thread::spawn(move || drop(options));
/// ```
///
/// Not `Clone`: the payloads its callbacks receive may be exclusive borrows, which a
/// copy would alias. Build another with [`new`](Self::new) instead.
///
/// Wraps: git_checkout_options
/// Handwritten
#[repr(transparent)]
pub struct CheckoutOptions<'o>(
    ffi::git_checkout_options,
    PhantomData<(*const (), &'o CStr)>,
);

/// Shared borrow of a [`CheckoutOptions`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct CheckoutOptionsRef<'a, 'o>(CBorrowedPtr<'a, CheckoutOptions<'o>>);

/// Exclusive borrow of a [`CheckoutOptions`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
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
    type Ref<'a>
        = CheckoutOptionsRef<'a, 'o>
    where
        Self: 'a;
    type Mut<'a>
        = CheckoutOptionsMut<'a, 'o>
    where
        Self: 'a;
}

impl<'o> CheckoutOptions<'o> {
    /// libgit2's default options, as `GIT_CHECKOUT_OPTIONS_INIT` spells them: version 1,
    /// a safe checkout (strategy 0), every other field zero or NULL.
    ///
    /// `git_checkout_options_init` only fails for a version other than
    /// `GIT_CHECKOUT_OPTIONS_VERSION`, and with that version it reaches no global, no
    /// allocator and no thread state, so this needs no initialised library.
    ///
    /// Wraps: git_checkout_options_init
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
    /// Borrows the options at `ptr`; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// `ptr` must address live, initialised options (or be NULL) that outlive `'a` and
    /// that nothing writes while the handle lives, whose strings, path list and
    /// baseline index outlive `'o` (strings NULL or NUL-terminated; `paths` as
    /// [`StrArrayRef`] requires; the index NULL or one an [`IndexRef<'o>`] may designate).
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_checkout_options) -> Option<Self> {
        let ptr = ptr::NonNull::new(ptr.cast::<CheckoutOptions<'o>>())?;
        // SAFETY: layout-preserving cast per `#[repr(transparent)]`; the caller upholds
        // liveness, the lifetimes and the absence of writers.
        Some(Self(unsafe { CBorrowedPtr::new(ptr) }))
    }

    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_checkout_options *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_checkout_options {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The structure's version, `GIT_CHECKOUT_OPTIONS_VERSION`; fixed by
    /// [`CheckoutOptions::new`].
    ///
    /// Field: git_checkout_options.version
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).version).read() }
    }

    /// The checkout strategy, the raw bits of `git_checkout_strategy_t`: 0, the default,
    /// is a safe checkout.
    ///
    /// Field: git_checkout_options.checkout_strategy
    pub fn strategy(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).checkout_strategy).read() }
    }

    /// Whether filters such as CRLF conversion are skipped when files are written; no
    /// by default.
    ///
    /// Field: git_checkout_options.disable_filters
    pub fn disable_filters(&self) -> bool {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        let disable = unsafe { addr_of!((*p).disable_filters).read() };
        disable != 0
    }

    /// The mode directories are created with; 0, the default, means `0755`.
    ///
    /// Field: git_checkout_options.dir_mode
    pub fn dir_mode(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).dir_mode).read() }
    }

    /// The mode files are created with; 0, the default, means the blob's own (`0644`
    /// or `0755`).
    ///
    /// Field: git_checkout_options.file_mode
    pub fn file_mode(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).file_mode).read() }
    }

    /// The `open(2)` flags files are written with; 0, the default, and any negative
    /// value mean `O_CREAT | O_TRUNC | O_WRONLY`.
    ///
    /// Field: git_checkout_options.file_open_flags
    pub fn file_open_flags(&self) -> i32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).file_open_flags).read() }
    }

    /// The situations the notification callback is called for, the raw bits of
    /// `git_checkout_notify_t`; none by default.
    ///
    /// Field: git_checkout_options.notify_flags
    pub fn notify_flags(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).notify_flags).read() }
    }

    /// The notification callback and its payload; `None` and NULL by default.
    ///
    /// Raw seam: the callback is `git_checkout_notify_cb`, which this crate does not
    /// wrap yet.
    ///
    /// Field: git_checkout_options.notify_cb
    /// Field: git_checkout_options.notify_payload
    pub fn notify_callback(&self) -> (ffi::git_checkout_notify_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        unsafe {
            (
                addr_of!((*p).notify_cb).read(),
                addr_of!((*p).notify_payload).read(),
            )
        }
    }

    /// The progress callback and its payload; `None` and NULL by default.
    ///
    /// Raw seam: the callback is `git_checkout_progress_cb`, which this crate does not
    /// wrap yet.
    ///
    /// Field: git_checkout_options.progress_cb
    /// Field: git_checkout_options.progress_payload
    pub fn progress_callback(&self) -> (ffi::git_checkout_progress_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        unsafe {
            (
                addr_of!((*p).progress_cb).read(),
                addr_of!((*p).progress_payload).read(),
            )
        }
    }

    /// The performance-data callback and its payload; `None` and NULL by default.
    ///
    /// Raw seam: the callback is `git_checkout_perfdata_cb`, which this crate does not
    /// wrap yet.
    ///
    /// Field: git_checkout_options.perfdata_cb
    /// Field: git_checkout_options.perfdata_payload
    pub fn perfdata_callback(&self) -> (ffi::git_checkout_perfdata_cb, *mut c_void) {
        let p = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to live options.
        unsafe {
            (
                addr_of!((*p).perfdata_cb).read(),
                addr_of!((*p).perfdata_payload).read(),
            )
        }
    }

    /// The paths, or wildmatch patterns, the checkout is restricted to; empty, the
    /// default, means every path. Borrowed from the options, since the array header is
    /// stored in them.
    ///
    /// Field: git_checkout_options.paths
    pub fn paths(&self) -> StrArrayRef<'a> {
        let p = self.as_ptr();
        // SAFETY: projects the embedded `git_strarray` of the live options, which outlive
        // `'a`. Every setter leaves it NULL with count 0, or a copy of an array a
        // `StrArrayRef<'o>` vouched for, whose strings outlive `'o` and so `'a`; nothing
        // writes it while this shared handle lives, and the result has no write
        // operation.
        let paths = unsafe { StrArrayRef::from_ptr(addr_of!((*p).paths).cast_mut()) };
        paths.expect("a field address is not NULL")
    }

    /// The baseline tree the working directory is compared against; NULL, the default,
    /// means `HEAD`'s (unless a [`baseline_index`](Self::baseline_index) is set).
    ///
    /// Raw seam: this crate does not wrap `git_tree` yet.
    ///
    /// Field: git_checkout_options.baseline
    pub fn baseline(&self) -> *mut ffi::git_tree {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).baseline).read() }
    }

    /// The baseline index, which overrides [`baseline`](Self::baseline); `None` by
    /// default. Borrowed for `'o`, as it was set.
    ///
    /// Field: git_checkout_options.baseline_index
    pub fn baseline_index(&self) -> Option<IndexRef<'o>> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or an `IndexRef<'o>`'s pointer, an index its holder keeps
        // live for `'o`, which this handle only lends on.
        unsafe { IndexRef::from_ptr(addr_of!((*p).baseline_index).read()) }
    }

    /// The directory files are written to; `None`, the default, means the repository's
    /// working directory.
    ///
    /// Field: git_checkout_options.target_directory
    pub fn target_directory(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).target_directory).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The name of the common ancestor's side in conflict markers; `None`, the
    /// default, means `"ancestor"`.
    ///
    /// Field: git_checkout_options.ancestor_label
    pub fn ancestor_label(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).ancestor_label).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The name of "our" side in conflict markers; `None`, the default, means
    /// `"ours"`.
    ///
    /// Field: git_checkout_options.our_label
    pub fn our_label(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).our_label).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The name of "their" side in conflict markers; `None`, the default, means
    /// `"theirs"`.
    ///
    /// Field: git_checkout_options.their_label
    pub fn their_label(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).their_label).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
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
            .field("baseline", &self.baseline())
            .field("baseline_index", &self.baseline_index().is_some())
            .field("target_directory", &self.target_directory())
            .field("ancestor_label", &self.ancestor_label())
            .field("our_label", &self.our_label())
            .field("their_label", &self.their_label())
            .field("perfdata_callback", &perfdata.is_some())
            .finish()
    }
}

impl<'a, 'o> CheckoutOptionsMut<'a, 'o> {
    /// Borrows the options at `ptr` exclusively; `None` if it is NULL.
    ///
    /// # Safety
    ///
    /// As [`CheckoutOptionsRef::from_ptr`], plus: nothing else reads or writes the
    /// options while the result lives, and whatever the setters store may stay there
    /// for as long as the options' owner expects `'o` borrows to.
    #[allow(dead_code)]
    pub(crate) unsafe fn from_ptr(ptr: *mut ffi::git_checkout_options) -> Option<Self> {
        // SAFETY: the caller upholds `CheckoutOptionsRef::from_ptr`'s contract, and
        // excludes every other access for `'a`.
        let shared = unsafe { CheckoutOptionsRef::from_ptr(ptr) }?;
        Some(Self(shared, PhantomData))
    }

    /// Writable pointer to the C struct, for the setters.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_checkout_options {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
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
    /// Field: git_checkout_options.checkout_strategy
    pub fn set_strategy(&mut self, strategy: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).checkout_strategy).write(strategy) }
    }

    /// Sets whether filters such as CRLF conversion are skipped when files are
    /// written.
    ///
    /// Field: git_checkout_options.disable_filters
    pub fn set_disable_filters(&mut self, disable: bool) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).disable_filters).write(c_int::from(disable)) }
    }

    /// Sets the mode directories are created with; 0 means `0755`.
    ///
    /// Field: git_checkout_options.dir_mode
    pub fn set_dir_mode(&mut self, mode: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).dir_mode).write(mode) }
    }

    /// Sets the mode files are created with; 0 means the blob's own.
    ///
    /// Field: git_checkout_options.file_mode
    pub fn set_file_mode(&mut self, mode: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).file_mode).write(mode) }
    }

    /// Sets the `open(2)` flags files are written with; 0 or a negative value means
    /// `O_CREAT | O_TRUNC | O_WRONLY`. Flags that cannot write the file (`O_EXCL` over
    /// an existing one, say) make the checkout fail with libgit2's error.
    ///
    /// Field: git_checkout_options.file_open_flags
    pub fn set_file_open_flags(&mut self, flags: i32) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).file_open_flags).write(flags) }
    }

    /// Sets the situations the notification callback is called for, the raw bits of
    /// `git_checkout_notify_t`.
    ///
    /// Field: git_checkout_options.notify_flags
    pub fn set_notify_flags(&mut self, flags: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).notify_flags).write(flags) }
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
    /// Field: git_checkout_options.notify_cb
    /// Field: git_checkout_options.notify_payload
    pub unsafe fn set_notify_callback(
        &mut self,
        callback: ffi::git_checkout_notify_cb,
        payload: *mut c_void,
    ) {
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to live options; the
        // fields are plain data, so the old values need no drop.
        unsafe {
            addr_of_mut!((*p).notify_cb).write(callback);
            addr_of_mut!((*p).notify_payload).write(payload);
        }
    }

    /// Sets the progress callback and the payload it is passed; `None` calls nothing.
    /// libgit2 reports each file it writes or removes, and the total.
    ///
    /// Raw seam: the callback is `git_checkout_progress_cb`, which this crate does not
    /// wrap yet.
    ///
    /// # Safety
    ///
    /// As [`set_notify_callback`](Self::set_notify_callback), for
    /// `git_checkout_progress_cb`'s arguments: the path may be NULL.
    ///
    /// Field: git_checkout_options.progress_cb
    /// Field: git_checkout_options.progress_payload
    pub unsafe fn set_progress_callback(
        &mut self,
        callback: ffi::git_checkout_progress_cb,
        payload: *mut c_void,
    ) {
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to live options; the
        // fields are plain data, so the old values need no drop.
        unsafe {
            addr_of_mut!((*p).progress_cb).write(callback);
            addr_of_mut!((*p).progress_payload).write(payload);
        }
    }

    /// Sets the performance-data callback and the payload it is passed; `None` calls
    /// nothing. libgit2 calls it once a checkout succeeds, with counts of the
    /// `mkdir`, `stat` and `chmod` calls it made.
    ///
    /// Raw seam: the callback is `git_checkout_perfdata_cb`, which this crate does not
    /// wrap yet.
    ///
    /// # Safety
    ///
    /// As [`set_notify_callback`](Self::set_notify_callback), for
    /// `git_checkout_perfdata_cb`'s arguments.
    ///
    /// Field: git_checkout_options.perfdata_cb
    /// Field: git_checkout_options.perfdata_payload
    pub unsafe fn set_perfdata_callback(
        &mut self,
        callback: ffi::git_checkout_perfdata_cb,
        payload: *mut c_void,
    ) {
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to live options; the
        // fields are plain data, so the old values need no drop.
        unsafe {
            addr_of_mut!((*p).perfdata_cb).write(callback);
            addr_of_mut!((*p).perfdata_payload).write(payload);
        }
    }

    /// Restricts the checkout to `paths`: wildmatch patterns, or exact paths with the
    /// `GIT_CHECKOUT_DISABLE_PATHSPEC_MATCH` strategy bit. libgit2 reads the array and
    /// its strings during each call, and copies what it keeps; an empty array means
    /// every path.
    ///
    /// The array is borrowed for `'o`, from a [`StrArrayView`] over borrowed strings or
    /// an owned [`StrArrayVal`](crate::libgit2::core::strarray::StrArrayVal), which can
    /// then not be changed while the options live.
    ///
    /// [`StrArrayView`]: crate::libgit2::core::strarray::StrArrayView
    ///
    /// Field: git_checkout_options.paths
    pub fn set_paths(&mut self, paths: StrArrayRef<'o>) {
        // A NULL array reads as empty whatever its count; C would read `count` strings.
        let count = paths.len();
        // SAFETY: a read through the shared handle's pointer to a live `git_strarray`.
        let strings = unsafe { addr_of!((*paths.as_ptr()).strings).read() };
        let raw = ffi::git_strarray {
            strings: if count == 0 { ptr::null_mut() } else { strings },
            count,
        };
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data. `raw` is NULL with count 0, or `count` non-NULL strings in
        // an array that `paths` vouches for for `'o`, as `paths()` requires.
        unsafe { addr_of_mut!((*p).paths).write(raw) }
    }

    /// Lifts the path restriction: every path is checked out.
    ///
    /// Field: git_checkout_options.paths
    pub fn clear_paths(&mut self) {
        let empty = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 0,
        };
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).paths).write(empty) }
    }

    /// Sets the baseline tree the working directory is compared against; NULL means
    /// `HEAD`'s. libgit2 only reads it, taking a reference of its own for each call.
    ///
    /// Raw seam: this crate does not wrap `git_tree` yet.
    ///
    /// # Safety
    ///
    /// `tree` must be NULL or point to a `git_tree` that stays live (its caller keeps a
    /// reference to it) until these options are dropped or the baseline is replaced.
    ///
    /// Field: git_checkout_options.baseline
    pub unsafe fn set_baseline(&mut self, tree: *mut ffi::git_tree) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).baseline).write(tree) }
    }

    /// Sets the baseline index, which overrides the [baseline
    /// tree](Self::set_baseline); `None` means none. libgit2 snapshots it for each call,
    /// counting a reference of its own for the call only.
    ///
    /// The index is borrowed for `'o` from the owner holding it, which then outlives the
    /// options, and an [`IndexBox`](crate::libgit2::index::index::IndexBox) lends no
    /// exclusive handle meanwhile.
    ///
    /// Field: git_checkout_options.baseline_index
    pub fn set_baseline_index(&mut self, index: Option<IndexRef<'o>>) {
        let index = index.map_or(ptr::null_mut(), |i| i.as_ptr().cast_mut());
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop. The index is NULL or one
        // an `IndexRef<'o>` vouches for, as `baseline_index()` requires.
        unsafe { addr_of_mut!((*p).baseline_index).write(index) }
    }

    /// Sets the directory files are written to, created when missing; `None` means
    /// the repository's working directory, which a bare repository lacks.
    ///
    /// Field: git_checkout_options.target_directory
    pub fn set_target_directory(&mut self, directory: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe {
            addr_of_mut!((*p).target_directory).write(directory.map_or(ptr::null(), CStr::as_ptr))
        }
    }

    /// Sets the name of the common ancestor's side in conflict markers; `None` means
    /// `"ancestor"`.
    ///
    /// Field: git_checkout_options.ancestor_label
    pub fn set_ancestor_label(&mut self, label: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).ancestor_label).write(label.map_or(ptr::null(), CStr::as_ptr)) }
    }

    /// Sets the name of "our" side in conflict markers; `None` means `"ours"`.
    ///
    /// Field: git_checkout_options.our_label
    pub fn set_our_label(&mut self, label: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).our_label).write(label.map_or(ptr::null(), CStr::as_ptr)) }
    }

    /// Sets the name of "their" side in conflict markers; `None` means `"theirs"`.
    ///
    /// Field: git_checkout_options.their_label
    pub fn set_their_label(&mut self, label: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).their_label).write(label.map_or(ptr::null(), CStr::as_ptr)) }
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

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::core::strarray::{StrArray, StrArrayView};
    use crate::libgit2::index::index::{Index, IndexArc};
    use core::ffi::c_char;
    use std::ffi::CString;

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
        assert!(o.baseline().is_null() && o.baseline_index().is_none());
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
        let strings = unsafe { addr_of!((*shared).strings).read() };
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
        let stale = unsafe { StrArrayRef::from_ptr(&mut stale) }.unwrap();
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
        let empty = unsafe { StrArrayRef::from_ptr(&mut raw) }.unwrap();
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
        let field = addr_of!(options.0.paths);
        assert_eq!(o.paths().as_ptr(), field);
        assert_eq!(o.paths().get(0), Some(c"a"));
        assert_eq!(o.paths().get(1), None);
    }

    #[test]
    fn raw_seams_round_trip() {
        let mut payloads = [0u8; 3];
        let [a, b, c] = payloads
            .each_mut()
            .map(|p| ptr::from_mut(p).cast::<c_void>());
        let mut options = template();
        let mut m = options.as_mut();
        // SAFETY: the callbacks ignore their arguments and are never called: no libgit2
        // routine sees these options; the payloads outlive them.
        unsafe {
            m.set_notify_callback(Some(notify), a);
            m.set_progress_callback(Some(progress), b);
            m.set_perfdata_callback(Some(perfdata), c);
        }
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
        unsafe {
            m.set_notify_callback(None, ptr::null_mut());
            m.set_progress_callback(None, ptr::null_mut());
            m.set_perfdata_callback(None, ptr::null_mut());
            m.set_baseline(ptr::null_mut());
        }
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
        assert!(options.as_ref().baseline().is_null());
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
        let shown = format!("{:?}", m);
        assert_eq!(shown, format!("{:?}", options));
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
            "baseline: 0x0",
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
}
