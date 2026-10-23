# Configuration entries and mapping records — wrap batch 0

## Analysis and representation

Verified the homogeneous `type` route and used the requested `config/config.rs`
home. Read the public declarations, backend extension layout, list ownership,
file/memory/snapshot implementations, field touchers and consumers. Queried
Wavefront's records, dependency closure, field touchers and lifecycle schema;
submitted both types and the entry acquisition/release symbols via `--update`.

* `Entry`/`EntryRef`/`EntryMut` use ffibox's layout/handle generator. A lookup
  acquires a reference to immutable backend-owned storage; repeated lookups may
  return the identical entry. `EntryArc` therefore exposes shared access only,
  without `Clone` or a sole-owner claim. `EntryFree` calls C's dynamic backend
  release callback through `git_config_entry_free`, never frees strings itself.
* Names and values belong to the retained list (names can be shared between
  multivars); backend labels and origins are interned in that list. All strings
  are immutable entry-lifetime borrows. Null and empty values remain distinct;
  custom priorities and unsigned include depths remain lossless. The built-in
  list survives refresh and config/repository destruction until entries release.
* `ConfigRef::entry` is the safe acquisition constructor, preserving error codes
  and messages. Callback/iterator entries use the shared borrowed form, not this
  ownership policy; their higher-layer wrappers are not yet available in the base.
  No public entry up-ref or standalone deep-copy constructor exists.
* `ConfigMap<'s>` is a handwritten transparent, resource-free layout implementing
  ffibox's `CCell`. It stores immutable string borrows, never an owned allocation.
  Shared handles are copyable; mutable handles are invariant in `'s` and reborrow
  exclusively. Rust construction/cloning need no C lifetime policy. `MapMatch`
  maintains the discriminator/string invariant: String requires a non-null string.
  Mode/string replacement is joint; an ignored string can also be stored for
  non-string modes. Integer mode's mapped scalar remains observable even though
  C lookup returns the parsed input instead. No `Send`/`Sync` claims were added.

There is no new public unsafe API or public raw argument/return. Raw adoption
fixtures and borrowed seams are tested internally only. Existing lower-layer
wrappers had no entry/map raw surface to replace. The sys allowlists gained only
the two record types and entry acquisition/release functions; bindings were
regenerated and the sys crate built/tested with the workspace. No C/shim changed.

## Reusable builds and validation

Compiled inputs match campaign C revision
`0551dfd4ad989b6a3d5683c0d4cf326c6efef929`; comparison against setup commit
`74e3b602a` found no C/header/dependency/CMake/build.json changes. Build configuration
is version 1. Cached compilers/flags match clang 18.1.3 for plain, coverage,
ASan/UBSan and TSan, and the prepared BSan compiler/plugin. Reused the immutable
`/target/crustify/builds` libraries with `cargo-variant.sh`; logs and Rust outputs
are agent-private. C coverage writes use `/tmp/config-b0-gcov` via `GCOV_PREFIX`.
A wrap-only batch without compiled C changes needs no C baseline rebuild.

Validation:

* `cargo check --workspace`, `cargo clippy --workspace`, and additional
  `cargo clippy --workspace --all-targets` pass.
* Plain, ASan/UBSan and TSan full-workspace tests pass: 598 unit, 375 equivalence,
  628 ub_safe, 91 ub_unsafe, and 280 doctests (42 merged + 238 compile-fail/other).
  One pre-existing unit test is ignored. The raw sys crate also passes.
* This batch adds two internal unit tests, two safe-UB integration tests, two
  equivalence integration tests and eight compile-fail doctests. Independent C
  and Rust inputs compare acquisition/errors, every field, refresh, retained old
  entries, release-after-parent-drop, mapping transitions and C dispatch.
* Compile-fail evidence rejects owner escape, stored-string escape, shared/mutable
  aliasing, mutable-lifetime shrinking, entry mutation/cloning, and thread transfer.
* Miri passes both new Rust-only unit tests and the public-safe mapping integration
  test. Foreign-call tests are explicitly ignored under Miri.
* BSan passes only these two exact tests, individually under 240-second timeouts:
  `config::entry_and_map_types::entries_retain_storage_across_parent_drop_and_refresh`
  and `config::entry_and_map_types::maps_preserve_borrows_across_clone_and_mutable_reborrows`
  in `ub_safe`. No binary-wide or workspace BSan run was performed.
* Static scan finds no unsafe/raw smell site or undocumented unsafe block in this
  batch's wrapper/test additions. Existing workspace smells are unchanged.
  `git diff --check` passes.

## Coverage

Ran the `config::` unit and safe/equivalence integration suites with the prepared
coverage C library and Rust llvm-cov instrumentation. Inspected Rust JSON coverage
and private C gcov files; both new unit tests fill the initially uncovered raw
borrow seams and nullable entry fields. Every authored type-wrapper method has
nonzero execution; the authored block has no zero-count executable region.
The entire shared Rust TU is approximately 99% covered (older wrappers included).

C execution includes `git_config_entry_free` (6/6 executable lines),
`git_config_get_entry` (2/2), its `get_entry` helper (25/28),
`git_config_list_entry_free` (4/4), list teardown (20/20), file acquisition (12/14),
and mapping dispatch (20/20). Remaining helper lines concern internal
error-suppression modes, unreachable through the public entry constructor.
File-backend refresh/list-acquisition failure branches are not forced. The memory
backend's acquisition path is unexercised: no safe memory-backend constructor is
available in the base, and its internal constructors are not exported by this DSO.
The Rust-only fixture covers its null-origin representation. Unknown mapping
modes and NULL String rules are excluded by the typed constructor/seam invariant;
no intentionally invalid FFI object is used to force their panic paths.
