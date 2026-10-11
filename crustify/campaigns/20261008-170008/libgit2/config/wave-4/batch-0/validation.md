# config wave 4 batch 0 — type / wrap

Workset: `git_config_backend`, homed in `config/sys/config.rs`. All fourteen
field anchors are filled. C layout and callback implementations remain C-owned.

## Ownership and safe boundary

- `Backend` is ffibox's transparent layout; `BackendRef` is copyable metadata
  access and `BackendMut` is exclusive dispatch. `BackendBox` uses a stateless
  allocation-recovered free policy. No `Send`, `Sync`, or bytewise `Clone`.
- Public initialization produces an inert inline template. Memory constructors
  produce `UnopenedBackend`; consuming `open` initializes once, releasing the
  complete allocation if parsing fails. Inputs and both option strings are copied.
  No consumer needs `assume_init`. Missing callbacks return errors before dispatch.
- `get` acquires a retained `BackendEntryArc`. Iterators borrow their backend
  exclusively and lend entries. Snapshots are opened synchronously while their
  source is live; their stored source pointer is never used thereafter.
- Writable `get` uses an independent snapshot rather than the defective file-get
  refresh-error path already documented by the earlier config entry wrapper.
- Callbacks cannot be detached or replaced through safe APIs. Open consumes only
  construction storage; free consumes only an owner; unlock consumes only a
  successful transaction. Transactions cannot reborrow an unrestricted backend
  and roll back on drop. Errors and unlock status are checked.
- The C `cfg` registration back-pointer may dangle after `open_level` shares a
  backend and the original config dies. Only an opaque, non-dereferenceable
  identity is exposed. Allocation-owned version/readonly metadata is inspected,
  not rewritten behind the allocator's invariants.
- The obsolete iterator raw seam is replaced by `backend() -> BackendRef` and a
  typed stored borrow. `set_backend` remains unsafe only for allocation-specific
  next/free compatibility and metadata immutability; liveness is now typed.
- C file open retains a repository. No repository-retaining constructor is exposed
  here; future such constructors require a lifetime-carrying owner. ffibox raw
  adoption is an internal interoperability seam with the documented full-allocation,
  callback, initialization, and external-dependency obligations.
- Malformed value pairs without `=` are rejected with `GIT_EINVALIDSPEC`: C's
  parser subtracts the NULL `strchr` result, so that path is not an equivalence oracle.

Wavefront ownership findings and constructor/init/free roles were submitted with
`--update`; no ownership-store edits were made by hand.

## Link prerequisite and builds

Both memory constructors were published with plain `extern`, but absent from the
shared library's exports. The two declarations now use `GIT_EXTERN(int)`; their
signatures, layout, and C bodies are unchanged. This lifts the campaign's previous
linkability exclusion only for these required backend lifecycle prerequisites.
No shim or Rust C export was added.

The shared builds at `/target/crustify/builds` were inspected, not modified. Because
this export annotation changes compiled C, all five variants were privately built
at `/tmp/crustify-config-w4-b0/<variant>` using `crustify/build.json` version 1,
its original compiler/instrumentation, and `-j12`. C inputs remained unchanged
across the rebase onto the other wave-4 batch. `validation.json` records their hash.

**Subsequent runners must rebuild C or select these private libraries**; the old
immutable shared libraries do not export the new constructors. Set
`CRUSTIFY_LIBGIT2_BUILD_DIR` and an agent-private `CARGO_TARGET_DIR`, then use
`crustify/rust/scripts/cargo-variant.sh <variant> ...`.

## Validation after rebase

- `cargo check --workspace`: passed.
- `cargo clippy --workspace --all-targets`: passed.
- `cargo test --workspace`, through the ASan/UBSan runner and matching private C:
  630 unit, 403 equivalence, 666 safe UB, 92 unsafe UB, 43 sys-layout, and 271
  doctests passed; one pre-existing unit ignored. Leak detection enabled.
- TSan: both backend safe tests, three backend equivalence tests, the typed setter
  unsafe test, and four backend internal tests passed. No thread-safety assertions
  were introduced.
- BSan: exact names only, not whole binaries: typed iterator setter; internal
  transaction/reborrow/release test; copied-input/retained-entry/snapshot test;
  values/options/failed-open test. All passed. These specifically exercise foreign
  writes and retained pointers across reborrows or input/parent destruction.
- Miri: Rust-only layout/stale identity and transaction/reborrow/drop tests passed;
  two foreign-call tests explicitly ignored. Borrow/outlive/reentry/clone/thread
  rejection is also checked by compile-fail doctests.
- `crustify scan-unsafe`: summary saved beside this file. Site scan covered public
  unsafe/raw smells, private raw smells, wrapper references, and projections. No
  public raw pointer was added. The sole touched public unsafe smell is the typed
  setter's unavoidable allocation-specific compatibility obligation. Other existing
  crate smells are outside this workset. Clippy verifies the SAFETY comments.

Private C baseline, Rust replacements absent:
`ctest --test-dir <variant> -j2 --output-on-failure -R '^(offline|util)$'`.
Plain, coverage, and ASan/UBSan passed both tests. TSan's offline test reported its
existing lock-order/race baseline; util passed. BSan's offline test stopped at the
known C `object::cache` bounds error; util passed. Isolated `object::cache` was run
on **both** the original reusable BSan library and the private library: both report
`test_object_cache__threadmania`, `tests/libgit2/object/cache.c:222`, a 4-byte access
outside an 8-byte allocation. This is unrelated to the two export annotations.

Agent-unique command logs, complete coverage profiles and C traces are retained
under `/tmp/crustify-config-w4-b0/`; no reusable-build output was overwritten.

## Coverage

Measured only this batch's unit, UB-safe, UB-unsafe and equivalence tests, with
`cargo llvm-cov` and the matching private coverage C library. Private C counters
were cleared after its baseline; no C baseline data contaminates these measurements.

- Backend Rust body: 185/187 distinct executable segment lines hit (98.93%). The
  two missing lines are runtime-pinning failure returns, dependent on initialization
  failure. The lending/mutable/transaction/owner paths and NULL-slot rejection were
  exercised, including rejection of a missing unlock before acquiring a lock.
- `config_mem.c`: 175/203 lines (86.21%).
- `config_snapshot.c`: 94/106 lines (88.68%).
- `git_config_init_backend` executes completely for the supported version. The
  rest of `config.c` is outside this workset, except its iterator fixture helpers.

Coverage inspection added snapshot missing-key/readonly operations, values-origin
metadata, positive transaction lifecycle, writable snapshot lookup, and absent-slot
checks. Remaining C gaps are allocator-failure cleanup, failed internal mutex
acquisition, iterator/copy failures dependent on allocation, NULL destructor calls
that non-null owners never issue, and readonly unlock callbacks that a successful
transaction cannot reach. Writable native file callbacks have no safely constructed
backend owner in this batch; their corresponding dispatch paths are exercised by
one allocation-matched Rust-owned mock fixture and the existing config subsystem.
