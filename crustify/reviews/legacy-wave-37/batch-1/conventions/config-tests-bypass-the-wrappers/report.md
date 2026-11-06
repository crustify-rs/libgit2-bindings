# Config tests still drove the public API's routines through raw FFI fixtures

- **items**: `git_config_get_int32`, `git_config_set_int32`,
  `git_config_add_file_ondisk` (symbols, wave 1 batch 1, `5a0f495b6`), which replaced
  the fixtures, and the `Config` owners the tests exercise
- **affected commit**: `d79771f6f`
- **class**: test convention (safe-API suites using raw stand-ins after the wrappers
  landed)

## Finding

The test rules say to build test objects through safe constructors, and to replace a raw
stand-in fixture "once the safe constructor exists". The UB suites test the safe public
API, and the equivalence suite compares C with "the public Rust API". Wave 0 had no
config routines, so it added stand-ins in `tests/ub_safe/fixture.rs::commit_and_config`:

- `get_int32(*mut git_config, name)` → `git_config_get_int32`;
- `set_int32(*mut git_config, name, value)` → `git_config_set_int32`;
- `write_through_file(*mut git_config, ..)` → `git_config_add_file_ondisk` +
  `git_config_set_int32`.

The fixture's doc says "replace each once its safe counterpart lands". Wave 1 batch 1
landed all three (`ConfigRef::get_i32`, `ConfigMut::set_i32`, `ConfigMut::add_file`),
but nothing was replaced:

- `tests/ub_safe/config.rs` (5 tests), `tests/ub_safe/transaction.rs`
  (`config_lock_borrows_and_abort_releases_backend`, whose comment still called the file
  backend "not-yet-wrapped") and `tests/ub_safe/repository.rs`
  (`installed_indexes_and_configs_outlive_or_predecease_the_repository`) still passed
  `owner.as_c_ptr()` to the raw fixtures. The `ub_safe/config.rs` module doc still said
  "The config routines have no wrappers yet".
- `repository_config_reparses_against_its_live_repository` wrote through a *shared*
  repository config with the raw `set_int32`. The safe API forbids that write, because a
  `ConfigArc` has no exclusive handle. So the test exercised a path no safe caller can
  reach.
- `tests/equiv/config.rs`'s `new_configs_match`, `shared_configs_match` and
  `repository_configs_match` ran one raw `steps` function on both sides, the Rust side
  through `as_c_ptr`. Those three tests compared C with C, and the wrappers they are
  meant to check were never called. The module doc also still said "no wrappers yet".

## Fix

- `tests/ub_safe/config.rs` reads and writes through `get_i32` / `set_i32` / `add_file`
  (two local helpers, `number` and `write_through_file`, over the safe API). The module
  doc now says the tests use the safe wrappers only. The includeIf test now triggers the
  reparse the way a safe caller can: it changes the file on disk and looks the value up
  through the shared handle. The lookup refreshes the backend, which evaluates
  `includeIf` against the live repository.
- `tests/ub_safe/transaction.rs` and `tests/ub_safe/repository.rs` use the wrappers. The
  three raw fixtures, and the imports only they used, are removed from `fixture.rs`.
- `tests/equiv/config.rs`: the write sequence is now `c_steps` (raw C) against
  `rust_steps` (safe wrappers on a `ConfigBox`). Each Rust result becomes the same
  record as C's: code, value, and the error the `ConfigError` carries. The shared case
  writes through the box before sharing it, then compares a read phase on the arc with
  C's. The repository case opens its Rust side with `Repository::open` and `config()`
  and reads with `get_i32`. The module doc describes the new scheme.

The `unit_tests` in `config.rs` and `refs/transaction.rs` still make a few raw calls.
Those suites test internal plumbing (raw adoption, a commit that has no wrapper yet),
which the rules allow.
