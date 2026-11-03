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

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;

use ffibox::{CMutSeam, CRefSeam, CZeroable, define_ctype};

use crate::ffi;

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

#[cfg(test)]
mod unit_tests {
    use std::collections::hash_map::DefaultHasher;

    use super::*;

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
