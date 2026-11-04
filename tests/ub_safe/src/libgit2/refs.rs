//! UB tests for `git2/refs.h`: the `git_reference` owners, `ReferenceBox<'r>` for an
//! attached reference and `UnattachedReferenceBox` for one a backend builds, and
//! their handles.
//!
//! Every owner releases its reference through `git_reference_free` exactly once, and
//! an attached one copies through `git_reference_dup`; a double free, a leaked copy
//! or a read of a freed reference is flagged by `ASan` (`LSan` included). Copies outlive
//! their originals here, chains of copies are dropped in every order, and C reads
//! each survivor back. The constructors copy their arguments (`name` into the
//! reference's flexible array, a symbolic target into its own allocation), so the
//! tests build references from Rust buffers that are overwritten or freed right after,
//! from names long enough that an off-by-one in the flexible array would leave the
//! allocation, and from ids of every type byte; libgit2 then retains no pointer into
//! Rust memory for a later foreign write (`BSan`).
//!
//! An unattached reference is `Send` and `Sync`: these tests build, share and drop
//! them across threads (`TSan`), and race the first constructors of a fresh process
//! to pin libgit2's runtime. An attached one is neither (doctests in `refs.rs`). A
//! handle or owner outliving its owner or repository borrow, aliased exclusive
//! handles, and a copy of an unattached reference do not compile (`compile_fail/`
//! and doctests).
//!
//! Every test calls libgit2, so Miri skips them all.
