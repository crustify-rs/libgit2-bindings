//! The pack indexer of `src/libgit2/indexer.c`.
//!
//! # Progress
//!
//! An [`IndexerProgress`] is a `git_indexer_progress`: the counters libgit2 keeps while it
//! indexes a packfile, whether it reads one directly or downloads one in a fetch or a clone.
//! Six of them count objects and deltas, as C's `unsigned int`; the seventh counts the bytes
//! received so far, as a `size_t`.
//!
//! It owns nothing, so it is held by value, copied freely and never released. libgit2 fills
//! one in storage its caller provides, which the caller zeroes first (`git_packbuilder_write`
//! and the smart transport's download do): when the indexer reads a pack's header it sets
//! `total_objects` to the header's count and resets the five other object and delta counters,
//! then advances them as it receives and hashes objects and resolves deltas, while
//! `received_bytes` is left to the transport that downloads the pack. A remote embeds the one
//! its last download filled, and a progress callback borrows one for the duration of the call.
//! The wrappers of those routines hand it out as an [`IndexerProgressRef`] or an
//! [`IndexerProgressMut`] into that storage.
//!
//! Every value is valid for the type: the counters are plain integers, and the relations
//! between them (no more indexed objects than received ones, say) are what the indexer
//! maintains, not what the type promises. A transport or a backend that reports progress
//! writes the counters through the setters.
//!
//! # Progress callbacks
//!
//! A [`ProgressCallback`] is a `git_indexer_progress_cb` over a Rust closure. libgit2 calls
//! one synchronously, on the thread that drives the indexing (an append to the indexer or
//! to an odb's writepack, a pack download), lending it the progress for the call alone, and
//! cancels the indexing or download when it returns a negative code. The closure returns
//! [`ControlFlow::Break`] to cancel, which the callback reports as `GIT_EUSER`: libgit2
//! documents only negative codes as cancelling, and some of its callers stop on any non-zero
//! code while the delta resolution goes on after a positive one, so a positive code is never
//! returned.

use core::any::Any;
use core::ffi::{c_int, c_void};
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::{fmt, ptr};
use std::panic::{self, AssertUnwindSafe};

use ffibox::{CMutSeam, CRefSeam, CZeroable, define_ctype};

use crate::ffi;
use crate::libgit2::src::util::errors::ErrorCode;

define_ctype!(
    /// The progress of indexing a packfile (`git_indexer_progress`): how many objects and
    /// deltas the pack holds, how many have been received, injected and indexed, and how
    /// many bytes have arrived.
    ///
    /// A plain value of 32 bytes, resource-free: `Copy`, held inline, never released.
    /// Start from [`IndexerProgress::default`] or
    /// [`IndexerProgress::zeroed`](ffibox::CZeroable::zeroed), every counter at 0, as callers
    /// zero one before handing it to libgit2. Read and write its counters through
    /// [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut).
    ///
    /// Wraps `ty::git_indexer_progress`
    #[doc(alias = "git_indexer_progress")]
    #[derive(Clone, Copy)]
    IndexerProgress,
    IndexerProgressRef,
    IndexerProgressMut,
    ffi::git_indexer_progress
);

// SAFETY: every field is a plain integer, so all zero is every counter at 0, a value every
// getter and setter accepts and libgit2 counts from (its callers zero a progress before handing
// it over: `git_packbuilder_write`, `git_smart__download_pack`), and there is no teardown.
unsafe impl CZeroable for IndexerProgress {}

// SAFETY: an `IndexerProgress` is 32 plain bytes: no pointer, no interior mutability, no thread
// affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for IndexerProgress {}

// SAFETY: `IndexerProgressMut` is an exclusive borrow of such a value, as
// `&mut IndexerProgress`.
unsafe impl Send for IndexerProgressMut<'_> {}

// SAFETY: shared access only reads the counters (every write goes through
// `IndexerProgressMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for IndexerProgress {}

// SAFETY: as above; `IndexerProgressRef` is a shared borrow, as `&IndexerProgress`.
unsafe impl Send for IndexerProgressRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for IndexerProgressRef<'_> {}

// SAFETY: `&IndexerProgressMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for IndexerProgressMut<'_> {}

/// Every counter at 0, as [`zeroed`](ffibox::CZeroable::zeroed) gives it.
impl Default for IndexerProgress {
    fn default() -> IndexerProgress {
        IndexerProgress::zeroed()
    }
}

/// The `git_indexer_progress` value as it is, for code that already holds one by value.
impl From<ffi::git_indexer_progress> for IndexerProgress {
    fn from(raw: ffi::git_indexer_progress) -> IndexerProgress {
        IndexerProgress(raw, PhantomData)
    }
}

/// The `git_indexer_progress` value, for code that passes one by value.
impl From<IndexerProgress> for ffi::git_indexer_progress {
    fn from(progress: IndexerProgress) -> ffi::git_indexer_progress {
        progress.0
    }
}

impl IndexerProgressRef<'_> {
    /// The number of objects in the packfile being indexed, as its header states it.
    ///
    /// Wraps `fld::git_indexer_progress.total_objects`
    #[doc(alias = "git_indexer_progress.total_objects")]
    #[must_use]
    pub fn total_objects(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let total_objects_ptr = unsafe { &raw const (*self.as_ptr()).total_objects };
        // SAFETY: a read through the shared handle's pointer to a live `git_indexer_progress`;
        // any value is a valid `u32`.
        unsafe { total_objects_ptr.read() }
    }

    /// The number of received objects that have been hashed.
    ///
    /// Wraps `fld::git_indexer_progress.indexed_objects`
    #[doc(alias = "git_indexer_progress.indexed_objects")]
    #[must_use]
    pub fn indexed_objects(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let indexed_objects_ptr = unsafe { &raw const (*self.as_ptr()).indexed_objects };
        // SAFETY: a read through the shared handle's pointer to a live `git_indexer_progress`;
        // any value is a valid `u32`.
        unsafe { indexed_objects_ptr.read() }
    }

    /// The number of objects that have been downloaded.
    ///
    /// Wraps `fld::git_indexer_progress.received_objects`
    #[doc(alias = "git_indexer_progress.received_objects")]
    #[must_use]
    pub fn received_objects(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let received_objects_ptr = unsafe { &raw const (*self.as_ptr()).received_objects };
        // SAFETY: a read through the shared handle's pointer to a live `git_indexer_progress`;
        // any value is a valid `u32`.
        unsafe { received_objects_ptr.read() }
    }

    /// The number of locally available objects injected to complete a thin pack.
    ///
    /// Wraps `fld::git_indexer_progress.local_objects`
    #[doc(alias = "git_indexer_progress.local_objects")]
    #[must_use]
    pub fn local_objects(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let local_objects_ptr = unsafe { &raw const (*self.as_ptr()).local_objects };
        // SAFETY: a read through the shared handle's pointer to a live `git_indexer_progress`;
        // any value is a valid `u32`.
        unsafe { local_objects_ptr.read() }
    }

    /// The number of deltas in the packfile being indexed. The indexer only knows it once
    /// every object has been received.
    ///
    /// Wraps `fld::git_indexer_progress.total_deltas`
    #[doc(alias = "git_indexer_progress.total_deltas")]
    #[must_use]
    pub fn total_deltas(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let total_deltas_ptr = unsafe { &raw const (*self.as_ptr()).total_deltas };
        // SAFETY: a read through the shared handle's pointer to a live `git_indexer_progress`;
        // any value is a valid `u32`.
        unsafe { total_deltas_ptr.read() }
    }

    /// The number of received deltas that have been resolved and indexed.
    ///
    /// Wraps `fld::git_indexer_progress.indexed_deltas`
    #[doc(alias = "git_indexer_progress.indexed_deltas")]
    #[must_use]
    pub fn indexed_deltas(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let indexed_deltas_ptr = unsafe { &raw const (*self.as_ptr()).indexed_deltas };
        // SAFETY: a read through the shared handle's pointer to a live `git_indexer_progress`;
        // any value is a valid `u32`.
        unsafe { indexed_deltas_ptr.read() }
    }

    /// The size in bytes of the packfile received so far.
    ///
    /// Wraps `fld::git_indexer_progress.received_bytes`
    #[doc(alias = "git_indexer_progress.received_bytes")]
    #[must_use]
    pub fn received_bytes(&self) -> usize {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let received_bytes_ptr = unsafe { &raw const (*self.as_ptr()).received_bytes };
        // SAFETY: a read through the shared handle's pointer to a live `git_indexer_progress`;
        // any value is a valid `usize`.
        unsafe { received_bytes_ptr.read() }
    }

    /// A copy of the counters, as a value of its own, which outlives the storage this handle
    /// borrows.
    #[must_use]
    pub fn to_progress(&self) -> IndexerProgress {
        IndexerProgress::from(ffi::git_indexer_progress {
            total_objects: self.total_objects(),
            indexed_objects: self.indexed_objects(),
            received_objects: self.received_objects(),
            local_objects: self.local_objects(),
            total_deltas: self.total_deltas(),
            indexed_deltas: self.indexed_deltas(),
            received_bytes: self.received_bytes(),
        })
    }
}

impl IndexerProgressMut<'_> {
    /// Replaces the number of objects in the packfile.
    ///
    /// Wraps `fld::git_indexer_progress.total_objects`
    #[doc(alias = "git_indexer_progress.total_objects")]
    pub fn set_total_objects(&mut self, total_objects: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let total_objects_ptr = unsafe { &raw mut (*self.as_mut_ptr()).total_objects };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_indexer_progress`; the field is a plain integer.
        unsafe { total_objects_ptr.write(total_objects) }
    }

    /// Replaces the number of hashed objects.
    ///
    /// Wraps `fld::git_indexer_progress.indexed_objects`
    #[doc(alias = "git_indexer_progress.indexed_objects")]
    pub fn set_indexed_objects(&mut self, indexed_objects: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let indexed_objects_ptr = unsafe { &raw mut (*self.as_mut_ptr()).indexed_objects };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_indexer_progress`; the field is a plain integer.
        unsafe { indexed_objects_ptr.write(indexed_objects) }
    }

    /// Replaces the number of downloaded objects.
    ///
    /// Wraps `fld::git_indexer_progress.received_objects`
    #[doc(alias = "git_indexer_progress.received_objects")]
    pub fn set_received_objects(&mut self, received_objects: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let received_objects_ptr = unsafe { &raw mut (*self.as_mut_ptr()).received_objects };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_indexer_progress`; the field is a plain integer.
        unsafe { received_objects_ptr.write(received_objects) }
    }

    /// Replaces the number of objects injected to complete a thin pack.
    ///
    /// Wraps `fld::git_indexer_progress.local_objects`
    #[doc(alias = "git_indexer_progress.local_objects")]
    pub fn set_local_objects(&mut self, local_objects: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let local_objects_ptr = unsafe { &raw mut (*self.as_mut_ptr()).local_objects };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_indexer_progress`; the field is a plain integer.
        unsafe { local_objects_ptr.write(local_objects) }
    }

    /// Replaces the number of deltas in the packfile.
    ///
    /// Wraps `fld::git_indexer_progress.total_deltas`
    #[doc(alias = "git_indexer_progress.total_deltas")]
    pub fn set_total_deltas(&mut self, total_deltas: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let total_deltas_ptr = unsafe { &raw mut (*self.as_mut_ptr()).total_deltas };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_indexer_progress`; the field is a plain integer.
        unsafe { total_deltas_ptr.write(total_deltas) }
    }

    /// Replaces the number of indexed deltas.
    ///
    /// Wraps `fld::git_indexer_progress.indexed_deltas`
    #[doc(alias = "git_indexer_progress.indexed_deltas")]
    pub fn set_indexed_deltas(&mut self, indexed_deltas: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let indexed_deltas_ptr = unsafe { &raw mut (*self.as_mut_ptr()).indexed_deltas };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_indexer_progress`; the field is a plain integer.
        unsafe { indexed_deltas_ptr.write(indexed_deltas) }
    }

    /// Replaces the number of bytes received.
    ///
    /// Wraps `fld::git_indexer_progress.received_bytes`
    #[doc(alias = "git_indexer_progress.received_bytes")]
    pub fn set_received_bytes(&mut self, received_bytes: usize) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let received_bytes_ptr = unsafe { &raw mut (*self.as_mut_ptr()).received_bytes };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_indexer_progress`; the field is a plain integer.
        unsafe { received_bytes_ptr.write(received_bytes) }
    }
}

impl fmt::Debug for IndexerProgressRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexerProgress")
            .field("total_objects", &self.total_objects())
            .field("indexed_objects", &self.indexed_objects())
            .field("received_objects", &self.received_objects())
            .field("local_objects", &self.local_objects())
            .field("total_deltas", &self.total_deltas())
            .field("indexed_deltas", &self.indexed_deltas())
            .field("received_bytes", &self.received_bytes())
            .finish()
    }
}

impl fmt::Debug for IndexerProgressMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for IndexerProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<IndexerProgressRef<'b>> for IndexerProgressRef<'_> {
    /// Counter by counter.
    fn eq(&self, other: &IndexerProgressRef<'b>) -> bool {
        self.total_objects() == other.total_objects()
            && self.indexed_objects() == other.indexed_objects()
            && self.received_objects() == other.received_objects()
            && self.local_objects() == other.local_objects()
            && self.total_deltas() == other.total_deltas()
            && self.indexed_deltas() == other.indexed_deltas()
            && self.received_bytes() == other.received_bytes()
    }
}

impl Eq for IndexerProgressRef<'_> {}

impl Hash for IndexerProgressRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.total_objects().hash(state);
        self.indexed_objects().hash(state);
        self.received_objects().hash(state);
        self.local_objects().hash(state);
        self.total_deltas().hash(state);
        self.indexed_deltas().hash(state);
        self.received_bytes().hash(state);
    }
}

impl PartialEq for IndexerProgress {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for IndexerProgress {}

impl Hash for IndexerProgress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// An `IndexerProgress` is a `git_indexer_progress`, so C reads and writes a value Rust holds in
// place.
const _: () = assert!(
    size_of::<IndexerProgress>() == size_of::<ffi::git_indexer_progress>()
        && align_of::<IndexerProgress>() == align_of::<ffi::git_indexer_progress>()
);

/// The closure behind a [`ProgressCallback`].
type ProgressFn<'f> = dyn FnMut(IndexerProgressRef<'_>) -> ControlFlow<()> + 'f;

/// A `git_indexer_progress_cb`: the callback libgit2 calls with the progress of indexing a
/// packfile, as it receives objects and resolves deltas, and as a download receives bytes.
///
/// Built over a closure with [`new`](Self::new). Its `extern "C"` trampoline lends the
/// closure the progress as an [`IndexerProgressRef`] valid for the call only (libgit2 lends
/// the counters it is updating), and returns [`ControlFlow::Continue`] as 0 and
/// [`ControlFlow::Break`] as `GIT_EUSER`, which cancels the indexing or the download: the
/// routine that drove it fails with that code. A panic in the closure does not cross into
/// C: the trampoline cancels with `GIT_EUSER`, every later call it gets cancels again
/// without running the closure, and the panic resumes once control is back in Rust, from
/// [`call`](Self::call) or from the wrapper that ran the indexing.
///
/// ```
/// use std::ops::ControlFlow;
///
/// use libgit2::libgit2::src::libgit2::indexer::{
///     IndexerProgress, IndexerProgressRef, ProgressCallback,
/// };
///
/// let mut seen = Vec::new();
/// let mut record = |progress: IndexerProgressRef<'_>| {
///     seen.push(progress.to_progress());
///     if progress.received_objects() < progress.total_objects() {
///         ControlFlow::Continue(())
///     } else {
///         ControlFlow::Break(())
///     }
/// };
/// let mut callback = ProgressCallback::new(&mut record);
/// let mut progress = IndexerProgress::default();
/// progress.as_mut().set_total_objects(2);
/// progress.as_mut().set_received_objects(1);
/// assert_eq!(callback.call(progress.as_ref()), ControlFlow::Continue(()));
/// progress.as_mut().set_received_objects(2);
/// assert_eq!(callback.call(progress.as_ref()), ControlFlow::Break(()));
/// drop(callback);
/// assert_eq!(seen.len(), 2);
/// assert_eq!(seen[1], progress);
/// ```
///
/// Wraps `ty::git_indexer_progress_cb`
#[doc(alias = "git_indexer_progress_cb")]
pub struct ProgressCallback<'f> {
    f: &'f mut ProgressFn<'f>,
    /// A panic the closure raised inside C, resumed once control is back in Rust.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> ProgressCallback<'f> {
    /// The callback that calls `f` with each progress, until it breaks.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(IndexerProgressRef<'_>) -> ControlFlow<()> + 'f,
    {
        Self { f, panic: None }
    }

    /// Calls the callback with `progress`, through the trampoline libgit2 calls, and
    /// returns what libgit2 would read from it: [`ControlFlow::Break`] for a cancel. A panic
    /// of the closure resumes here.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic but by resuming the closure's panic: the trampoline is never NULL"
    )]
    pub fn call(&mut self, progress: IndexerProgressRef<'_>) -> ControlFlow<()> {
        let rc = self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the call;
            // `progress` a live progress, lent for the call, which only reads it: the
            // trampoline's contract.
            unsafe { callback(progress.as_ptr(), payload) }
        });
        if rc == 0 {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    }

    /// Runs `walk` with the C callback and its payload, as the wrapper of a routine that
    /// calls the callback (`git_odb_write_pack`'s writepack, `git_indexer_append`) hands
    /// them to it, then resumes a panic the closure raised meanwhile. The payload is this
    /// callback, which nothing else touches until `walk` returns; libgit2 must be done
    /// calling it by then.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_indexer_progress_cb, *mut c_void) -> R,
    ) -> R {
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

/// `git_indexer_progress_cb` over a [`ProgressCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`ProgressCallback`] that nothing else uses during the
/// call, and `stats` be NULL or a `git_indexer_progress` live and unchanged for the call.
unsafe extern "C" fn progress_with(
    stats: *const ffi::git_indexer_progress,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for the
    // call alone.
    let callback = unsafe { &mut *payload.cast::<ProgressCallback<'_>>() };
    if callback.panic.is_some() {
        return ErrorCode::User.raw();
    }
    // SAFETY: per this function's contract, a non-NULL `stats` is live and unchanged for
    // the call; the closure only reads it, through a handle that cannot outlive the call.
    let Some(progress) = (unsafe { IndexerProgressRef::from_ptr(stats.cast_mut()) }) else {
        // libgit2 always lends its counters; cancel rather than invent a progress.
        return ErrorCode::User.raw();
    };
    match panic::catch_unwind(AssertUnwindSafe(|| (callback.f)(progress))) {
        Ok(ControlFlow::Continue(())) => 0,
        Ok(ControlFlow::Break(())) => ErrorCode::User.raw(),
        Err(panic) => {
            callback.panic = Some(panic);
            ErrorCode::User.raw()
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use std::collections::hash_map::DefaultHasher;
    use std::ffi::CString;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::{fs, process};

    use super::*;
    use crate::libgit2::src::libgit2::odb::{Odb, OdbArc};
    use crate::libgit2::src::util::errors;

    fn raw(seed: u32) -> ffi::git_indexer_progress {
        ffi::git_indexer_progress {
            total_objects: seed,
            indexed_objects: seed.wrapping_add(1),
            received_objects: seed.wrapping_add(2),
            local_objects: seed.wrapping_add(3),
            total_deltas: seed.wrapping_add(4),
            indexed_deltas: seed.wrapping_add(5),
            received_bytes: usize::MAX - seed as usize,
        }
    }

    fn hash(progress: IndexerProgressRef<'_>) -> u64 {
        let mut h = DefaultHasher::new();
        progress.hash(&mut h);
        h.finish()
    }

    #[test]
    fn default_is_every_counter_at_zero() {
        let progress = IndexerProgress::default();
        assert_eq!(progress, IndexerProgress::zeroed());
        let r = progress.as_ref();
        assert_eq!(
            (
                r.total_objects(),
                r.indexed_objects(),
                r.received_objects(),
                r.local_objects(),
                r.total_deltas(),
                r.indexed_deltas(),
                r.received_bytes(),
            ),
            (0, 0, 0, 0, 0, 0, 0)
        );
    }

    #[test]
    fn raw_values_convert_both_ways_unchanged() {
        for seed in [0, 1, u32::MAX - 5, u32::MAX] {
            let progress = IndexerProgress::from(raw(seed));
            let r = progress.as_ref();
            assert_eq!(r.total_objects(), seed);
            assert_eq!(r.indexed_objects(), seed.wrapping_add(1));
            assert_eq!(r.received_objects(), seed.wrapping_add(2));
            assert_eq!(r.local_objects(), seed.wrapping_add(3));
            assert_eq!(r.total_deltas(), seed.wrapping_add(4));
            assert_eq!(r.indexed_deltas(), seed.wrapping_add(5));
            assert_eq!(r.received_bytes(), usize::MAX - seed as usize);
            let back = ffi::git_indexer_progress::from(progress);
            let expected = raw(seed);
            assert_eq!(
                (
                    back.total_objects,
                    back.indexed_objects,
                    back.received_objects,
                    back.local_objects,
                    back.total_deltas,
                    back.indexed_deltas,
                    back.received_bytes,
                ),
                (
                    expected.total_objects,
                    expected.indexed_objects,
                    expected.received_objects,
                    expected.local_objects,
                    expected.total_deltas,
                    expected.indexed_deltas,
                    expected.received_bytes,
                )
            );
        }
    }

    #[test]
    fn to_progress_copies_every_counter() {
        let progress = IndexerProgress::from(raw(7));
        let copy = progress.as_ref().to_progress();
        assert_eq!(copy, progress);
        assert_eq!(
            ffi::git_indexer_progress::from(copy).received_bytes,
            usize::MAX - 7
        );
    }

    #[test]
    fn equality_and_hash_follow_every_counter() {
        let base = IndexerProgress::from(raw(3));
        assert_eq!(base, IndexerProgress::from(raw(3)));
        assert_eq!(
            hash(base.as_ref()),
            hash(IndexerProgress::from(raw(3)).as_ref())
        );
        // The value hashes as its shared handle does.
        let mut h = DefaultHasher::new();
        base.hash(&mut h);
        assert_eq!(h.finish(), hash(base.as_ref()));
        // Changing any single counter makes the values differ.
        let setters: [fn(&mut IndexerProgressMut<'_>); 7] = [
            |m| m.set_total_objects(100),
            |m| m.set_indexed_objects(100),
            |m| m.set_received_objects(100),
            |m| m.set_local_objects(100),
            |m| m.set_total_deltas(100),
            |m| m.set_indexed_deltas(100),
            |m| m.set_received_bytes(100),
        ];
        for set in setters {
            let mut other = base;
            set(&mut other.as_mut());
            assert_ne!(other, base);
            assert_ne!(other.as_ref(), base.as_ref());
        }
    }

    /// A scratch directory of this process for `tag`, created empty.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("indexer-unit-{tag}-{}", process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(dir.join("objects")).unwrap();
        dir
    }

    /// A pack holding `blobs`, from `git pack-objects`, over SHA-1 ids.
    fn git_pack(dir: &Path, blobs: &[Vec<u8>]) -> Vec<u8> {
        let repo = dir.join("git");
        let git = |args: &[&str]| {
            let mut cmd = Command::new("git");
            cmd.args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", dir);
            cmd
        };
        let repo = repo.to_str().unwrap();
        assert!(
            git(&["init", "-q", "--bare", repo])
                .status()
                .unwrap()
                .success()
        );
        let mut ids = Vec::new();
        for blob in blobs {
            let mut child = git(&["-C", repo, "hash-object", "-w", "--stdin"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(blob).unwrap();
            let out = child.wait_with_output().unwrap();
            ids.push(String::from_utf8(out.stdout).unwrap().trim().to_owned());
        }
        let mut child = git(&["-C", repo, "pack-objects", "-q", "--stdout"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(ids.join("\n").as_bytes())
            .unwrap();
        child.wait_with_output().unwrap().stdout
    }

    /// Three blobs, two of them alike enough for `git pack-objects` to store one as a
    /// delta of the other.
    fn blobs() -> Vec<Vec<u8>> {
        let base: Vec<u8> = (0..4096u32)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        let mut edited = base.clone();
        edited.extend_from_slice(b"one more line\n");
        vec![base, edited, b"hello".to_vec()]
    }

    /// The results of indexing a pack through a writepack.
    struct Run {
        /// Each `append` member's code, in order.
        appends: Vec<c_int>,
        /// The `commit` member's code, if every append succeeded.
        commit: Option<c_int>,
        /// The progress the writepack filled.
        stats: IndexerProgress,
    }

    /// Opens a writepack on `odb` with the C callback and payload `walk_with` hands out,
    /// appends `pack` in pieces of `piece` bytes until one fails, commits if none did,
    /// and frees the writepack.
    fn index(
        odb: &OdbArc<'_>,
        pack: &[u8],
        piece: usize,
        callback: ffi::git_indexer_progress_cb,
        payload: *mut c_void,
    ) -> Run {
        let mut wp = ptr::null_mut();
        // SAFETY: `wp` is a writable slot; the odb is live, and locks itself while its
        // backends open the writepack; the callback and its payload stay valid until the
        // writepack is freed below, within the caller's `walk_with`.
        let rc = unsafe {
            ffi::git_odb_write_pack(
                &raw mut wp,
                odb.as_ref().as_ptr().cast_mut(),
                callback,
                payload,
            )
        };
        assert_eq!(rc, 0);
        let mut stats = IndexerProgress::zeroed();
        // SAFETY: `wp` is the live writepack libgit2 opened, with its members set.
        let members = unsafe { wp.read() };
        let (append, commit, free) = (
            members.append.unwrap(),
            members.commit.unwrap(),
            members.free.unwrap(),
        );
        let mut appends = Vec::new();
        for chunk in pack.chunks(piece) {
            // SAFETY: a member of the live `wp`; `chunk` is read for the call, `stats` is
            // a live progress written for the call only.
            let rc = unsafe {
                append(
                    wp,
                    chunk.as_ptr().cast(),
                    chunk.len(),
                    stats.as_mut().as_mut_ptr(),
                )
            };
            appends.push(rc);
            if rc != 0 {
                break;
            }
        }
        let commit = (appends.iter().all(|&rc| rc == 0)).then(|| {
            // SAFETY: as for `append`.
            unsafe { commit(wp, stats.as_mut().as_mut_ptr()) }
        });
        // SAFETY: `wp` is live and no longer used.
        unsafe { free(wp) };
        Run {
            appends,
            commit,
            stats,
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "builds a pack with git: file system isolation")]
    fn libgit2_calls_the_closure_with_its_progress() {
        let dir = scratch("progress");
        let pack = git_pack(&dir, &blobs());
        let objects = CString::new(dir.join("objects").to_str().unwrap()).unwrap();
        let odb = Odb::open(&objects).unwrap();
        let mut seen = Vec::new();
        let mut record = |progress: IndexerProgressRef<'_>| {
            seen.push(progress.to_progress());
            ControlFlow::Continue(())
        };
        let mut callback = ProgressCallback::new(&mut record);
        let run = callback.walk_with(|cb, payload| index(&odb, &pack, 97, cb, payload));
        drop(callback);
        assert!(run.appends.iter().all(|&rc| rc == 0));
        assert_eq!(run.commit, Some(0));
        let last = run.stats.as_ref();
        assert_eq!(last.total_objects(), 3);
        assert_eq!(last.indexed_objects(), 3);
        assert_eq!(last.received_objects(), 3);
        assert_eq!(*seen.last().unwrap(), run.stats);
        // The counters only grow, call after call.
        for pair in seen.windows(2) {
            let (a, b) = (pair[0].as_ref(), pair[1].as_ref());
            assert!(a.received_objects() <= b.received_objects());
            assert!(a.indexed_objects() <= b.indexed_objects());
            assert!(a.indexed_deltas() <= b.indexed_deltas());
        }
        // The edited blob is stored as a delta, resolved and reported at commit.
        assert_eq!(last.total_deltas(), 1);
        assert_eq!(last.indexed_deltas(), 1);
        assert!(seen.iter().any(|p| p.as_ref().indexed_deltas() == 1));
        drop(odb);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "builds a pack with git: file system isolation")]
    fn a_break_cancels_the_indexing_with_git_euser() {
        let dir = scratch("break");
        let pack = git_pack(&dir, &blobs());
        let objects = CString::new(dir.join("objects").to_str().unwrap()).unwrap();
        let odb = Odb::open(&objects).unwrap();
        let mut calls = 0;
        let mut stop = |_: IndexerProgressRef<'_>| {
            calls += 1;
            ControlFlow::Break(())
        };
        let mut callback = ProgressCallback::new(&mut stop);
        let run = callback.walk_with(|cb, payload| index(&odb, &pack, 4096, cb, payload));
        drop(callback);
        assert_eq!(calls, 1);
        assert_eq!(*run.appends.last().unwrap(), ErrorCode::User.raw());
        assert_eq!(run.commit, None);
        assert_eq!(
            errors::last().message(),
            c"indexer progress callback returned -7"
        );
        drop(odb);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "builds a pack with git: file system isolation")]
    fn a_panic_cancels_then_resumes_after_the_walk() {
        let dir = scratch("panic");
        let pack = git_pack(&dir, &blobs());
        let objects = CString::new(dir.join("objects").to_str().unwrap()).unwrap();
        let odb = Odb::open(&objects).unwrap();
        let mut calls = 0;
        let mut boom = |_: IndexerProgressRef<'_>| -> ControlFlow<()> {
            calls += 1;
            panic!("boom");
        };
        let mut callback = ProgressCallback::new(&mut boom);
        let mut codes = Vec::new();
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            callback.walk_with(|cb, payload| {
                let first = index(&odb, &pack, 4096, cb, payload);
                codes.push(*first.appends.last().unwrap());
                // A later call within the same walk cancels again, without the closure.
                let cb = cb.unwrap();
                let progress = IndexerProgress::zeroed();
                // SAFETY: `payload` is the callback `walk_with` lends; `progress` is live.
                codes.push(unsafe { cb(progress.as_ref().as_ptr(), payload) });
            });
        }));
        assert_eq!(*caught.unwrap_err().downcast::<&str>().unwrap(), "boom");
        assert_eq!(codes, [ErrorCode::User.raw(), ErrorCode::User.raw()]);
        assert!(format!("{callback:?}").contains("panicked: false"));
        drop(callback);
        assert_eq!(calls, 1);
        drop(odb);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_null_progress_cancels_without_the_closure() {
        let mut calls = 0;
        let mut count = |_: IndexerProgressRef<'_>| {
            calls += 1;
            ControlFlow::Continue(())
        };
        let mut callback = ProgressCallback::new(&mut count);
        let rc = callback.walk_with(|cb, payload| {
            // SAFETY: `payload` is the callback `walk_with` lends; NULL is in the
            // trampoline's contract.
            unsafe { cb.unwrap()(ptr::null(), payload) }
        });
        drop(callback);
        assert_eq!(rc, ErrorCode::User.raw());
        assert_eq!(calls, 0);
    }

    #[test]
    fn call_maps_control_flow_and_lends_the_progress() {
        let mut progress = IndexerProgress::from(raw(11));
        let mut seen = Vec::new();
        let mut f = |p: IndexerProgressRef<'_>| {
            seen.push(p.to_progress());
            if seen.len() == 1 {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        };
        let mut callback = ProgressCallback::new(&mut f);
        assert_eq!(callback.call(progress.as_ref()), ControlFlow::Continue(()));
        progress.as_mut().set_received_bytes(1);
        assert_eq!(callback.call(progress.as_ref()), ControlFlow::Break(()));
        assert_eq!(
            format!("{callback:?}"),
            "ProgressCallback { panicked: false, .. }"
        );
        drop(callback);
        assert_eq!(seen, [IndexerProgress::from(raw(11)), progress]);
    }

    #[test]
    fn debug_names_every_counter() {
        let mut progress = IndexerProgress::zeroed();
        let mut m = progress.as_mut();
        m.set_total_objects(10);
        m.set_received_bytes(4096);
        let expected = "IndexerProgress { total_objects: 10, indexed_objects: 0, \
                        received_objects: 0, local_objects: 0, total_deltas: 0, \
                        indexed_deltas: 0, received_bytes: 4096 }";
        assert_eq!(format!("{m:?}"), expected);
        assert_eq!(format!("{progress:?}"), expected);
        assert_eq!(format!("{:?}", progress.as_ref()), expected);
    }
}
