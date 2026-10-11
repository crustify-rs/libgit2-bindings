# Index scalar types — wave 0, batch 1

## Analysis and representation

- Route verified: two public enum types, `git_index_add_option_t` and
  `git_index_capability_t`, both defined in `include/git2/index.h`. Wavefront
  reports no fields, dependency closure or lifecycle operations. Source inspection
  covered enum constants, their C callers, `git_index_add_all`,
  `index_apply_to_wd_diff`, `git_index_set_caps`, and `git_index_caps`. Findings were
  submitted through `wavefront query types --update`.
- Both are copied, resource-free integers. There are no pointers, borrowed forms,
  ownership transfers, constructors, destructors or C clone routines to bind.
  `AddFlags` and `Capabilities` are handwritten transparent newtypes over the
  generated scalar aliases, not ffibox object owners or borrowed handles.
- `AddFlags` preserves every bit and combination. C currently tests the entire
  flags value for equality with `FORCE` when including ignored files; the wrapper
  documents this and does not normalize combinations or change that C behavior.
- Explicit `Capabilities` accepts only the published mask (0..=7). The signed
  `FROM_OWNER = -1` command is separately represented by
  `CapabilitySetting::FromOwner`; a compile-fail doctest proves it cannot be ORed
  into an explicit set. Unknown explicit bits are rejected by the safe constructor.
- No existing Rust consumer used these scalar types, so there was no lower-layer
  raw surface to replace. No public unsafe API was added. The only added FFI
  functions (`git_index_caps`, `git_index_set_caps`) support equivalence tests;
  their safe symbol wrappers remain for their scheduled batch.

## Build reuse

Reused the immutable campaign libraries via `cargo-variant.sh`. `build.json` is
version 1; C sources, headers, dependencies and CMake inputs match the shared
post-`c5440d3ee` rebuild. CMake caches confirm Clang 18.1.3 and the configured plain,
coverage, ASan/UBSan and TSan flags (BSan uses its configured toolchain). No C or
compiled shim changed; no full C baseline rerun was needed. Only this sys crate's allowlists were extended; bindgen
regenerated automatically, and `libgit2-sys` built/tested under ASan/UBSan.

Logs and detailed coverage reside in `/tmp/crustify-index-batch1/`; Rust outputs
are worktree-private. `GCOV_PREFIX=/tmp/crustify-index-batch1/gcov` redirected C
coverage writes away from the shared build; the gcno used for inspection was
copied, not changed in place.

## Validation

- `cargo check --workspace` and `cargo clippy --workspace` through the plain runner.
- `cargo test --workspace` through the ASan/UBSan runner, including all doctests
  and compile-fail cases: 645 unit passes (1 existing ignored), 410 equivalence,
  676 UB-safe, 92 UB-unsafe, and 317 doctests, with zero failures.
- TSan: the new scalar unit, UB-safe and equivalence tests, including scoped
  shared readers and moved copies. There are no handwritten Send/Sync impls.
- Miri: both new unit tests and both UB-safe tests; no foreign calls.
- BSan: no test selected. This pointer-free workset introduces no foreign
  reborrows, retained pointers, or conflicting writes expected to trigger BSan.
  No workspace-wide BSan run was made.
- `crustify scan-unsafe` with unsafe-function, raw-pointer and wrapped-reference
  smell sites: no smell belongs to either new type. There is no new unsafe block
  in production code. All equivalence-only FFI calls have local safety comments.

## Coverage

Final Rust coverage: **19/19 functions and 69/69 regions** in the new production
surface (union of the unit and integration test binaries). All executable lines
of the additionally exercised C capability consumers ran: `git_index_set_caps`
94% blocks and `git_index_caps` 100% blocks. See `coverage-summary.json`.

Coverage-instrumented tests exercise all flag combinations, all capability sets,
unknown and extreme add-option bits, rejected capability bits and commands,
operators, conversions, default/copy/clone/debug, and both setting variants.
C equivalence cases compare independently owned indexes after repeated explicit
updates, then the owner command both without an owner (failure and unchanged
state) and with an owner (successful config read).

The two enum declarations have no executable C bodies. C consumer coverage is
inspected additionally for `git_index_set_caps` and `git_index_caps`. Null-index
assertion failures cannot arise from the safe constructors; failing configuration
lookups are delegated repository/config behavior, not implemented by these types.
`git_index_add_all` itself is not wrapped by this batch: its enum values and every
combination are compared against generated C constants without duplicating its
later symbol wrapper or its filesystem behavior tests.
