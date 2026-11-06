# config wave 0 batch 0: type / wrap

## Analysis and surface

- `git_config_backend_memory_options`: caller-owned inline layout; the two nullable
  NUL strings are immutable caller borrows. `config_backend_new` copies them with
  `git__strdup`; neither memory-backend constructor retains the options or their
  string pointers. Backend destruction frees copies, not these borrows. Options
  initialization copies a template; there is no allocating constructor, disposer,
  dropper or C cloner for the options. Rust shallow cloning shares immutable borrows.
- `BackendMemoryOptions<'o>` uses a transparent layout and ffibox `CCell` /
  `CBorrowedPtr` handles. The lifetime parameter cannot be expressed by
  `define_ctype!`, hence the documented handwritten layout. Getters project raw
  fields; setters require strings borrowed for `'o`; mutable handles are invariant
  in that lifetime. NULL, empty, and non-UTF-8 strings remain distinct. Version is a
  scalar, read and replaced without interpreting it. No `Send` or `Sync` is asserted.
- `git_config_level_t`: resource-free signed scalar. `ConfigLevel` preserves custom
  integer priorities and the distinct `HIGHEST=-1` query selector without enum
  transmutation. Its docs flag signed subtraction in C's priority comparators:
  future backend-registration wrappers must validate their input ranges.
- Wavefront type findings and the initializer's constructor/argument findings were
  submitted with `--update`; dependency closures and lifecycle facets were queried.
  Wavefront also canonically reordered existing records. Semantic comparison against
  the base confirms only these two type records and one initializer record were added,
  with no existing record changed or removed.
- Only the required options type, initializer and version constant were added to the
  affected `-sys` allowlists. No C, header, shim, build configuration or implementation
  was changed. The initializer anchor allows its later symbol batch to skip duplication.

## Validation

Reusable `crustify/builds` libraries and `rust/scripts/cargo-variant.sh` were used.
The C tree matches campaign revision `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`
(`git diff` of include/src/deps/CMakeLists.txt/cmake is empty). Build version is 1;
CMake caches match the configured compiler/instrumentation (Clang 18.1.3, or the
configured BSan compiler). Shared C builds were not modified. Logs and coverage
outputs are agent-private under `/tmp/config-b0`; C coverage was redirected with
`GCOV_PREFIX`, with `.gcno` notes copied to that output tree for inspection.

Passed:

- `cargo check --workspace`, `cargo clippy --workspace`, `cargo test --workspace`
  through the plain runner, including bindgen regeneration and `libgit2-sys` tests.
- Full workspace tests with matching ASan/UBSan and TSan C/Rust instrumentation.
- Exact BSan invocation only for the added foreign-initialization/reborrow test:
  `--test ub_safe sys_config::memory_options_reborrow_and_replace -- --exact`.
- Miri: `--lib config::` (5 Rust-only passes, 5 existing foreign-call tests ignored)
  and exact public `config::config_level_values_and_threads` (one pass).
- Static `crustify scan-unsafe <worktree> --json --sites all`: no unsafe/raw smells
  or undocumented unsafe sites in the new surface. The only new unsafe function is
  a crate-private layout adoption seam; no public unsafe API was added, so there is
  no new `ub_unsafe` suite.

Added evidence: two safe runtime integration tests, four compile-fail doctests,
two equivalence integration tests, and three inline unit tests. They cover
construction/defaults, clones, shared and exclusive handles, replacement/reset,
non-UTF-8 and empty strings, signed/custom levels, thread-safe scalar copies,
layout/alignment and private pointer adoption (including NULL refusal).

## Coverage and limits

Combined focused coverage runs used the unit `config::` tests and the four new
runtime integration tests. `config/sys/config.rs` has 103/103 covered executable
lines (including its inline unit tests) and 19/19 covered functions; all added level conversion methods are covered
across the unit/integration instantiations. The entire existing `config/config.rs`
TU has 230/249 lines covered (92.37%); its remaining gaps are pre-existing config
surface outside this worklist.

C `git_config_backend_memory_options_init` has 5/5 executable lines covered,
returning success. Its assertion failure (NULL) and unsupported-version failure
are unreachable from the safe constructor, which always supplies exclusive valid
storage and the generated current version. The enclosing memory-backend TU has
uncovered operations outside this worklist.

Direct supplementary backend-consumer tests were attempted but not retained:
`git_config_backend_from_string` and `git_config_backend_from_values` are declared
with plain `extern`, not `GIT_EXTERN`; `nm -D` confirms they are absent from the
shared library's dynamic exports, while `nm` shows local `t` definitions. They
cannot be linked through the reusable library. Ownership conclusions for those
consumers therefore come from source inspection, not runtime claims. Their future
symbol wrappers must address this export limitation; no unrelated C export change
was made in this type batch.

## Integration retry

The first local push was rejected as non-fast-forward after batch 2 landed.
Only the agent branch was rebased onto `62c089598`; shared config source/test
conflicts were resolved by retaining both batches' complete additions. After the
rebase, workspace check/clippy/test, ASan/UBSan, TSan, the exact BSan test, the
static scan, and focused coverage were rerun. Post-rebase Miri reran the two
options unit tests, the level unit test, and the public level-thread test, all
successfully. The figures above are from the post-rebase coverage run. The final
ownership-store diff adds exactly three records without reordering prior records.
