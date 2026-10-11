# Config mapping modes and transactions — wrap batch 1

## Analysis and representation

Verified the homogeneous `type` route: `git_configmap_t` is an enum and
`git_transaction` is an opaque public struct. Both requested Rust homes are used.
Read the declarations, implementation, field touchers, lifecycle routines and
callers, using Wavefront records and source. Submitted two type records and four
lifecycle-symbol records through Wavefront's `--update`; existing records are
unchanged. Rebased onto both other batches before final validation; resolved the
ownership-store conflict by selecting the integration version and resubmitting
only this batch's findings through the CLI.

* `MapType` represents the four named mapping modes, uses generated constants at
  the C boundary, and rejects unknown raw values. It owns no resources. C's integer
  mode parses the input rather than returning the entry's `map_value`.
* Reference transactions live inside their own C pool. Their `repo` is a retained
  **borrow**, not a reference-counted owner; `db` holds an acquired refdb reference,
  released after outstanding locks. Both the refdb and its backend refer to the
  repository. `TransactionBox<'r>` therefore borrows that repository. Shared and
  exclusive handles preserve the borrow, and mutable reborrows are invariant.
* Config transactions are standalone C allocations. `cfg` is a retained borrow;
  `cfg_data` is specifically a `backend_instance *`, not an arbitrary payload.
  `git_config_lock` locks that backend and increments its reference count;
  commit/free unlocks and decrements it. Both pointers become NULL after commit.
* Pointer fields are scalar and nullable across the two discriminator variants.
  Node names, messages, signatures, symbolic targets and reflog contents are copied
  into the transaction's C pool; opaque reference-lock payloads are released by the
  refdb backend. Rust does not take over any of this storage.
* `TransactionFree` is a stateless ffibox `CDrop` policy. C's discriminator selects
  the correct allocator and pending-lock teardown. No transaction clone exists,
  and no `Send`/`Sync` assertion is made. The public struct publishes no fields, so
  no field accessors or concrete-body binding/shim are needed.
* `Transaction::new` pairs C construction with that policy. `ConfigMut::lock`
  returns a separate `ConfigTransaction`, retaining the exclusive config handle
  and the same ffibox owner internally. It never exposes reference-operation
  handles or a public mutable config handle: the latter would allow recursive
  locking of C's non-reentrant file-buffer state. Its config getter is borrowed
  from the transaction. The later commit-symbol wrapper must consume the owner;
  this batch does not expose commit or other non-lifecycle transaction symbols.

The lifetime-carrying layout/handles and config keepalive use documented
`Handwritten` representations against ffibox's `CCell` contract. Every owner is
constructed through a safe constructor; raw adoption tests are internal only.
There is no new public unsafe API (ffibox's generic escape hatches are unchanged).
The sys allowlists add only required lifecycle bindings and regression-test
bindings, and the affected sys crate was regenerated, built and tested.

## Reusable C builds

C sources, headers, vendored dependencies and CMake inputs match campaign revision
`0551dfd4ad989b6a3d5683c0d4cf326c6efef929`; `crustify/build.json` is version 1.
CMake caches/compiler records match clang 18.1.3 for plain, coverage, ASan/UBSan
and TSan, and the prepared clang 23.1.3 BSan plugin build. Reused the immutable
`/target/crustify/builds` libraries through the supplied `cargo-variant.sh` runner.
C coverage writes use an agent-private `GCOV_PREFIX`; logs and Rust build outputs
are agent-private. No compiled C/shim changed, so no C baseline rebuild is needed.

## Tests and static safety

Final-tree validation after the last rebase:

* `cargo check --workspace`, `cargo clippy --workspace`, and additionally clippy
  with `--all-targets`: passed. The all-targets run reports three existing
  `drop_non_drop` warnings in batch 0's memory-options tests; none concern this
  batch.
* Plain workspace tests and full ASan/UBSan workspace tests passed: 591 unit,
  371 equivalence, 625 ub_safe, 91 ub_unsafe, and 271 doctests (42 merged and
  229 other doctests); the raw sys test binary passed with zero runtime tests. One existing
  unit test is ignored. The batch's three safe and three equivalence tests and
  four internal unit tests are included.
* Compile-fail tests prevent parent-first drops, a handle outliving its owner,
  mutable/shared reborrow aliasing, aliasing the locked config and recursive
  config locking. Doctests also reject lifetime widening and `Send`/`Sync`.
* BSan: only these four exact names, separately under 180-second timeouts, passed:
  `transaction::config_lock_borrows_and_abort_releases_backend` (ub_safe),
  `transaction::reference_owners_and_reborrows_release_once` (ub_safe),
  `transaction::config_construct_stage_abort_and_relock` (equiv), and
  `libgit2::refs::transaction::unit_tests::config_commit_clears_release_state`
  (lib). They cover retained foreign parents, handle reborrows, staged foreign
  writes, abort and committed teardown. No workspace-wide BSan gate was run.
* Miri passed the opaque layout/handle-reborrow unit test, checked mapping-mode
  unit test and public-safe mapping-mode integration test. Foreign-call cases are
  explicitly ignored under Miri; the layout test never calls C or dereferences a
  fake C body.
* `crustify scan-unsafe . --json --sites all` passed. No new unsafe/raw smell sites
  occur in the workset; transaction implementation and equivalence tests have
  zero undocumented unsafe blocks. All layout operations and FFI calls have
  specific safety comments. `git diff --check` passed.

Equivalence inputs have independent transactions, configs, repositories/refdbs
and filesystem paths. Multi-call comparisons cover lock creation, failure codes
and normalized error messages, staged writes, abort, relocking, unmodified commit
and destruction. All four mapping modes are compared against C, including errors.

TSan's final full workspace run passed with `--test-threads=1`, while tests that
spawn worker threads internally still do so. An earlier parallel full-workspace
attempt intermittently exited 66 in existing equivalence tests:
`types::reference_types_follow_c` called `git_reference_create` /
`git_reference__log_signature` / `current_time`, racing libc `tzset_internal`'s
free with `git_signature_default` / `current_time`'s strdup from
`commit::create_options::Side::run_rust`. Neither stack contains this workset.
Other parallel full runs passed. This unrelated timezone report was not fixed or
classified as a workset defect; the trace is in the agent-private
`/tmp/crustify-config-batch-1/final-tsan.log`.

## Coverage and remaining C-owned paths

Ran transaction tests plus the mapping-mode internal test against the prepared
coverage library, redirected C gcda files, inspected Rust LCOV and copied C gcno
files with `llvm-cov-18 gcov -f -b` for the workset's C functions.

| C function | Executed lines |
|---|---:|
| `git_transaction_config_new` | 11/11 |
| `git_transaction_new` | 14/20 |
| `git_transaction_free` | 18/18 |
| `git_config_lock` | 11/11 |
| `git_config_unlock` | 8/8 |
| `git_config_lookup_map_value` | 20/20 |

Rust's transaction TU has 16/16 functions and 102/105 executable lines covered;
all authored mapping conversion/lock methods are exercised. The three remaining
Rust lines are the two runtime-pin failure returns and the C reference-constructor
failure return. Safe parent constructors already pin the runtime; allocation and
backend-setup failure injection were not included. C's six reference-constructor
lines cover allocation/backend failure cleanup, plus a pool-init failure that is
unreachable with the prepared non-debug pool and valid arguments.

Added a pre-existing-lock fixture to cover lock failure/recovery without damaging
another writer's lock, and internal NULL-release/committed-config cases to cover
all destructor lines. No reference update/reflog wrappers are authored by this
batch: their bodies and corresponding commit/error paths remain later-symbol
work, not claims of full transaction-TU coverage.

One C-owned error-path limitation remains: `git_config_lock` acquires its backend
lock before allocating the config transaction and has no public rollback path if
that allocation fails. Rust has no returned owner or backend-instance pointer to
unwind that path; this wrap batch preserves C's implementation. That allocation
failure path was not forced in the sanitizer tests and is not claimed covered.

All final logs, scan output and coverage data are under the agent-private
`/tmp/crustify-config-batch-1/land-*` paths. The C libraries remain unchanged.
