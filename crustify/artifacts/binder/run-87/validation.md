# Index wave 1, batch 1 — type / wrap

## Contracts and scope

- `git_index_reuc_entry`: entries are index-owned. `reuc_entry_alloc` callocs the
  struct with the path copied into the same block (`reuc_entry_internal`), so `path`
  is never NULL, never written after allocation, and has no separate free. Modes are
  the raw `uint32_t` from `git_index_reuc_add` (negative `int` modes wrap) or the
  on-disk octal; an id is copied (and written) only for a non-zero mode, otherwise the
  slot stays the typeless all-zero `GIT_OID_NONE`. `index_entry_reuc_free` releases
  the block on same-path replacement (`index_reuc_on_dup`), remove, clear (also via
  read/read_tree/clear and checkout's REUC update) and index free. The public API
  publishes no standalone constructor, clone or destructor, nor a writable entry.
- `ReucEntry` / `ReucEntryRef` / `ReucEntryMut` use ffibox's canonical layout and
  handles. Getters on `ReucEntryRef`: `path() -> &'a CStr`, `modes() -> [u32; 3]`,
  `ids() -> CSlice<'a, Oid>` (one `Field:` anchor each), plus typed
  `ancestor()/ours()/theirs() -> Option<ReucSide<'a>>` that hide slots with mode 0,
  and a Rust-only `Debug`. No `Send`/`Sync`; no public unsafe API, so no `ub_unsafe`
  suite.
- Reachability: `IndexMut::reuc_entry` (`git_index_reuc_get_byindex`) and
  `IndexMut::reuc_entry_by_path` (`git_index_reuc_get_bypath`) are emitted here so that
  the type has a safe source. They take `&self` on the exclusive handle: both C
  routines only read the vector (get_bypath returns before searching an unsorted
  vector), so several records may be held at once while every mutation is excluded.
  `IndexRef` lends no records, because an `IndexArc`'s other holders may mutate. These
  two filled `Wraps:` anchors must be skipped by wave 2 batch 0.
- Not emitted: `git_index_reuc_add/entrycount/find/remove/clear`, which the concurrent
  wave 1 batch 3 owns. Tests reach them only through `libgit2::ffi`
  (`tests/ub_safe/fixture.rs` `index::{add_reuc, remove_reuc, clear_reuc,
  set_ignore_case}`, documented stand-ins to replace once batch 3 lands), or through
  the safe `Index::open` of hand-written REUC index files (`tests/support/reuc.rs`).
- Wavefront: submitted the `path` field block and type note, the lifecycle roles of
  `index_entry_reuc_free` (dropper), `reuc_entry_alloc` and `index_entry_reuc_init`
  (alloc constructors), and the pointer contracts of both getters. The store diff is
  insertions only.
- C behaviour noted, not changed: `read_reuc` trusts on-disk order
  (`git_vector_set_sorted(true)` without sorting), so an out-of-order file breaks path
  lookups without UB. A record whose three modes are all 0 cannot be the last one
  read ("reading reuc entry stage"). On its `git_oid_from_raw` failure (unreachable
  with a valid index oid type) and on vector-insert OOM it returns without freeing the
  record it was filling (leak).

## Build reuse

No C, header, dependency, CMake or shim change; the worktree's `src/` and `include/`
match `/target`, `build.json` is version 1, and the shared builds' CMake caches name
clang (ASan/UBSan, TSan, coverage) and the BSan toolchain clang. Only bindgen
allowlists changed: type `git_index_reuc_entry`, functions `git_index_reuc_get_byindex`
and `get_bypath`, and `add/entrycount/remove/clear` for test fixtures. Reused the
immutable `crustify/builds` libraries through `crustify/scripts/cargo-variant.sh`
with agent-private target, log and gcov directories under `/tmp/crustify-index-w1b1/`.

## Validation

- `cargo check --workspace`, `cargo clippy --workspace --all-targets` (plain) and
  `asan-ubsan check/clippy --workspace`: clean apart from the existing manifest
  warnings.
- `asan-ubsan test --workspace` (LSan on): all pass, including the three new
  compile-fail doctests (E0502 mutation while a record is borrowed, E0505 index drop
  while a path is borrowed, E0599 no records from a shared handle) and the E0277
  not-Send doctest.
- TSan: the batch's unit, `ub_safe` and `equiv` tests (`sys_index`, `reuc`), passed.
- BSan: only exact `sys_index::reuc_entries_from_file_borrow_index`, under a timeout:
  passed. It holds 49 C-owned records through simultaneous shared handles, reads
  paths, modes and both id widths, and copies past the index's release.
- Miri: the Rust-only layout/side-projection unit tests passed; foreign calls are
  excluded.
- `crustify scan-unsafe`: no smell site in `index/sys/index.rs`; undocumented-unsafe
  total unchanged (45) after binding a split `let … = unsafe` in the equivalence test.

## Coverage

Coverage variant with Rust `-Cinstrument-coverage` and private `GCOV_PREFIX`, on the
batch's tests:

- Rust: every REUC line in `index/sys/index.rs` runs. The only unrun region is
  `side`'s `?` on `CSlice::get`, which cannot miss for slots 0–2. The file's uncovered
  lines are the earlier NAME tests, which the filter skipped.
- C: `git_index_reuc_get_byindex`, `get_bypath`, `find`, `remove`, `clear`,
  `entrycount`, `index_entry_reuc_init`, `index_reuc_insert`, `index_reuc_on_dup`,
  `index_entry_reuc_free` and all four comparators: 100% of lines. `read_reuc`: 90.7%.
  The malformed-payload test covers all four refusal paths. Unrun: vector init
  (already initialised by every constructor), `git_oid_from_raw` failure (unreachable
  with a valid oid type), and vector-insert OOM. `reuc_entry_alloc` (92.9%) and
  `git_index_reuc_add` (90.9%) miss only their allocation-failure arms. No allocator
  failure was injected, since it belongs to the batch 3 add wrapper.
