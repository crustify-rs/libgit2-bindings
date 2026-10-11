# Index wave 0, batch 5 — type / wrap

## Contracts and scope

- `git_index_time`: an `i32` seconds / `u32` nanoseconds pair embedded in index
  entries, copied and serialized verbatim (no normalization, every bit pattern
  valid, no pointers or lifecycle). `IndexTime` is the `define_ctype!` value itself
  (`CZeroable`, `new`, `Default`, `Clone`), with `IndexTimeRef::{seconds,
  nanoseconds, to_time}` and `IndexTimeMut::{set_seconds, set_nanoseconds}`.
  `git_index_entry` is not wrapped yet, so no consumer had a raw timestamp surface
  to replace.
- `git_iterator`: published only as the opaque `git_note_iterator`. Hand-written
  `NoteIterator<'r>` (lifetime-generic, as `ConflictIterator`), `NoteIteratorRef` /
  `NoteIteratorMut`, owner `NoteIteratorBox<'r>` released by `NoteIteratorFree`
  (`git_note_iterator_free`). C owns the root and frame trees; the repository is an
  uncounted borrow carried by `'r`. No clone, refcount or `Send`/`Sync`. Private
  fields and other iterator kinds are outside the wrap surface. Traversal
  (`git_note_next`) is left to its symbol batch; `as_ptr`/`as_mut_ptr` are its seam.
- Constructors `NoteIterator::new` and `NoteIterator::from_commit` carry the
  `git_note_iterator_new` / `git_note_commit_iterator_new` anchors; the symbol route
  must skip them and the releaser.
- No C implementation, ABI or shim changed. Findings submitted through Wavefront
  (`git_index_time`, `git_iterator`, the three lifecycle symbols).

## C defects, refused in Rust

All three free an uninitialised pointer; the C sources are left unchanged (wrap).

1. `git_note_commit_iterator_new`: `git_tree *tree;` is uninitialised and
   `git_tree_free(tree)` runs when `git_commit_tree` fails (notes commit whose tree
   is missing, OOM). Shown with a stale-stack C probe against the ASan build: gdb
   stops in `git_tree_free (obj=0x7fffffffe200)` from `notes.c:763`, a stack
   address that is then refcount-decremented. `from_commit` looks the tree up first
   (`CommitRef::tree`) and holds it over the C call, whose lookup then hits the cache
   or rereads the object. Residual: allocation failure or removal of the tree from
   the object database between the two lookups.
2. `git_note_iterator_new`: `retrieve_note_commit` returns `error` (0) when
   `git_commit_lookup` fails (reference to a missing object, a blob, a tree), so
   `git_note_commit_iterator_new(it, NULL)` fails the commit assertion ("invalid
   argument: 'commit'", confirmed in gdb) and reaches the same free.
3. With a NULL name, `note_get_default_ref` → `git_config__get_string_buf` frees the
   entry `get_entry` never wrote when the configuration file stops parsing after it
   was loaded (SIGSEGV in `git_config_entry_free`, `config.c:1009`; same root as the
   config getters' known `get_entry` defect).

`new` is therefore composed: `core.notesRef` through `ConfigRef::entry` (else
`refs/notes/commits`, valueless read as empty), `git_reference_name_to_id` (raw seam
until refs wraps it; added to the allowlist), `Commit::lookup`, then `from_commit`.
Success and every C-safe failure match C, including the error's code, class and
message. Divergence only where C is undefined: Rust returns the lookup's own error.
`RepositoryError::from_parts` (crate-private) carries a config failure across.

## Build reuse

Reused the orchestrator's immutable `crustify/builds` variants and
`scripts/cargo-variant.sh`. No change under `include`, `src`, `deps`, CMake inputs
or shims, so the C build matches; only bindgen allowlists changed. Logs, Rust
targets and gcov output stay in this worktree or `/tmp`
(`GCOV_PREFIX`).

## Validation

- `cargo check --workspace`, `cargo clippy --workspace`: clean.
  `cargo clippy --workspace --all-targets` reports one undocumented unsafe block at
  `tests/equiv/index.rs:1178`, already on the wave tip and not from this batch.
- `cargo test --workspace` (plain): all pass except trybuild
  `tests/ub_safe/compile_fail/diff_not_sync.rs`, a stderr mismatch (std source
  snippet missing under stable) already on the wave tip. It passes in the nightly
  ASan run.
- ASan/UBSan + LSan: `cargo-variant.sh asan-ubsan test --workspace --no-fail-fast`
  is fully green, compile-fail doctests included.
- TSan: every notes test (unit, `ub_safe`, `equiv`) and the timestamp tests,
  including four iterators built and released on threads at once.
- BSan, exact names only: `notes::construction_release_and_reborrows`,
  `notes::non_commit_targets_are_refused`, `index::timestamp_extremes_and_reborrows`.
  All pass.
- Miri: `unit_tests::value_clone_and_borrowed_projection` and
  `index::timestamp_extremes_and_reborrows` pass; foreign calls are excluded.
- Compile-fail doctests check four things: a timestamp handle cannot alias or
  outlive its value, an iterator cannot outlive its repository, the owner is not
  `Send`, and exclusive reborrows cannot alias.
- No public `unsafe fn` was added, so there is no `ub_unsafe` suite.
- `crustify scan-unsafe --sites all`: no smell site in the changed code. The one
  smell site in `repository.rs` (line 3483) predates this batch.

## Coverage

Coverage build with private gcov and `cargo llvm-cov` over the batch's tests.

- `notes.rs`: 179/179 lines, 15/15 functions, 425/427 regions. The two regions left
  are the error arm of `repo.config()` in `default_ref`: opening a repository already
  loads its configuration, so a later parse failure reaches `entry` instead (which is
  covered). `index.rs` timestamp block: fully covered.
- C `git_note_commit_iterator_new`: 6/7 lines. The missed line is the defect path
  (tree lookup failure), which the wrapper deliberately never reaches.
  `git_iterator_free` and `tree_iterator_free`: 100%. `git_reference_name_to_id`:
  100%.
- `git_iterator_for_tree`: 14/18 lines. The iterator allocation failure is covered by
  an isolated deterministic refusal of the 336-byte `tree_iterator` (C releases the
  tree, LSan clean). Not covered: `tree == NULL` (never passed) and `on_error`, which
  needs a `realloc` failure in the frame array that the refusal allocator does not
  inject.
- `git_note_iterator_free`: its NULL early return is unreachable, since owners are
  never null.

Local detailed output: `crustify/tmp/index-b5/` (ignored, retained in worktree).
