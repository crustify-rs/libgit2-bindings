# Index wave 0, batch 4 — type / wrap

## Contracts and scope

- `git_index_name_entry`: C allocates a record and duplicates up to three nullable
  NUL strings in `git_index_name_add`; the NAME reader also allocates records and
  treats empty on-disk strings as absent. The static `index_name_entry_free`
  destroys all three strings and the record. Index clear/reload/free owns teardown.
  Checkout and serialization only read the fields. There is no published standalone
  owner, clone, destructor, or writable entry-returning API.
- `NameEntry` uses ffibox's canonical layout/handles. Only `NameEntryRef` is obtained
  by safe code. All three field getters return optional C strings borrowing the
  exclusive index access that yielded the entry. No `Send`/`Sync` assertions.
- Parent-owned construction, lookup, counting and disposal are wrapped here as
  `IndexMut::{add_name,name_entry,clear_names}` / `IndexRef::name_entry_count`.
  Lookup takes exclusive access because C sorts lazily. These four filled symbol
  anchors must be skipped by their subsequent symbol batch.
- `git_index_stage_t`: signed wildcard -1 is distinct from stored stages 0–3.
  `Stage` preserves all five constants and rejects unknown values on conversion.
  No pointer or resource lifecycle. No C implementation or ABI was changed.
- Findings submitted through Wavefront for both types, the four public NAME
  operations, and the internal entry releaser. No existing Rust consumer had a
  raw NAME-entry surface to replace.

## Build reuse

Reused the orchestrator's immutable `crustify/builds` libraries and variant runner.
The C tree and `build.json` v1 match preparation revision `d8c416f22` and `/target`;
there are no changes in `include`, `src`, `deps`, CMake inputs, or compiled shims.
CMake caches confirm clang 18 and the configured ASan/UBSan, TSan and coverage
flags; BSan uses its configured toolchain/plugin. Only bindgen allowlists changed.
The sys crate was regenerated and tested against ASan/UBSan. Logs and Rust targets
are private to this worktree. Coverage uses `GCOV_PREFIX` so it never writes into
shared C build directories.

## Validation

- `cargo check --workspace` and `cargo clippy --workspace` (plain variant).
- `cargo test --workspace` against matching ASan/UBSan C and Rust, including
  compile-fail doctests that reject clear/drop with a surviving entry/string.
- TSan: NAME integration tests, scalar stage tests, and isolated NAME allocation
  failure test. No added unsafe public API, hence no added `ub_unsafe` suite.
- BSan: only exact `sys_index::names_survive_inputs_and_reborrow_before_clear`,
  under a 300-second timeout; passed. This drives input copying, foreign vector
  growth/sort, handle reborrows, clear/reuse, and owner release.
- Miri: Rust-only nullable field projection/layout fixture and stage tests passed;
  foreign calls are excluded.
- `crustify scan-unsafe` plus enumerated unsafe/raw smells: no changed production
  site appears among the smell listings; all new unsafe blocks have safety proofs.

## Coverage

Prepared C coverage library plus Rust `-Cinstrument-coverage`, using private gcov
and profraw output. Ran the batch's focused tests, then the index-filtered tests
for allocation failures and parser-produced nullable variants; inspected both
llvm-cov/gcov reports.

- NAME Rust TU: 77/77 executable lines, 9/9 functions, 135/136 regions (includes
  inline tests). The remaining region is the `clear_names` error arm: current C
  returns zero for every valid non-null index. Both Stage conversion bodies run,
  including all named and unknown raw values.
- C `git_index_name_add`, `name_clear`, `name_entrycount`, `name_get_byindex`,
  `conflict_name_cmp`, and `git_index_entry_stage`: 100% executable lines.
- `index_name_entry_free`: 7/8 lines; its null-input early return is unreachable
  through the public APIs, which only pass allocated records.
- `read_conflict_names`: 18/21 lines. Its vector-initialization fallback cannot be
  reached through normal index constructors (already initialized); NAME-reader
  vector-insertion allocation failure is not separately injected. Successful
  parsing covers all eight nullability combinations, and sorting covers parser
  records that `name_add` cannot construct.
- Added deterministic allocation refusal for NAME creation, each string copy, and
  vector growth, checking failure leaves existing entries unchanged and every
  partial allocation is released. No allocator failures rely on machine pressure.

Local detailed output: `crustify/tmp/index-b4/` (ignored, retained in worktree).
