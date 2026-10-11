# `BackendEntryFree` lacks its lifecycle anchors

- **item**: `git_config_backend_entry` (type), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `a6e60a4d5` (libgit2/config wave 2 batch 0)
- **class**: conventions (anchors)

## Finding

`BackendEntryFree` is the release policy of `BackendEntryArc`. It hand-implements
`CDrop<BackendEntry>`, and C's `git_config_entry_free` runs on every drop. Yet the
policy carried no anchor. The conventions ask for an anchor on the lifecycle routine a
`Drop` calls ("Emit anchors for the raw/typed lifecycle routines when implementing the
Drop/Clone that calls them"). Duplicates are allowed when several wrappers
intentionally represent one item, as `EntryFree` (for `Entry`) and `BackendEntryFree`
(for the enclosing layout) do. The ffibox guidance asks for `/// Handwritten` on
hand-written lifecycle code. The crate follows both everywhere else: every hand-written
`CDrop` policy (`RefdbFree`, `ConfigFree`, `TransactionFree`, `ReferenceFree`, …)
carries `Wraps: <routine>` and `Handwritten`. So the campaign's anchor and hand-written
accounting both missed this policy.

The impl also reached C one policy removed. It called `EntryFree.c_drop` through
`NonNull::new_unchecked` on the projected field, so its SAFETY argument depended on
another policy's contract rather than on the C routine it actually invokes.

## Fix

- `BackendEntryFree` carries `/// Wraps: git_config_entry_free` and `/// Handwritten`.
- `c_drop` calls `ffi::git_config_entry_free(addr_of_mut!((*entry).entry))` directly.
  Its SAFETY comments now state the offset-zero cast C performs and the single
  reference the callback consumes. The now-unused `Entry` and `EntryFree` imports are
  gone.

ffibox's alternative for a field-taking destructor, an `unsafe fn` adapter passed to
`impl_cdrop!` (as `WriteStreamFree` does in `core/types.rs`), was tried and rejected.
`crustify scan-unsafe` counts such a free-standing adapter as four smell sites
(`unsafe_fns_priv_smell`, `raw_ptr_priv_smell`, `field_proj_outside_impl`,
`raw_ptr_derefs_outside_impl`), while the trait impl's `c_drop` counts as a seam. The
final tree has no smell site in the workset's files. Versus the wave tip, the scan
shows `ffi_calls` +1 (this direct call) and `ref_to_type_wrapper_sanctioned` +1 (the
new `Debug` impl's `&self` on the layout type).

Behaviour is unchanged. `acquired_callback_and_owner_roundtrip` still counts exactly one
callback per release, including a release made reentrantly from inside a callback, and
the ub_safe and equivalence lifecycles pass under ASan/UBSan (LSan on) and TSan.
