# `cargo-variant.sh` sanitizes host proc-macros, so `asan-ubsan` and `tsan` fail to build

- **item**: `git_error_last` (symbol) — the review's UB obligation for the ASan/UBSan
  and TSan instruments, blocked by the shared runner
  `crustify/scripts/cargo-variant.sh`
- **affected commit**: `68ad24171` (wave-1 base; the runner is unchanged since
  `99533715f`, the raw-lifetime-void review)
- **class**: misc — a defect in campaign tooling, not in the workset's Rust; it
  prevents the sanitizer runs the UB step requires
- **reproducer**: none packaged; the command below fails deterministically on the
  reviewed tree

## Finding

The UB step of this review must exercise `last()` under every prepared instrument.
Running the ASan/UBSan runner on the error suites fails at build time:

```
$ crustify/scripts/cargo-variant.sh asan-ubsan test -p libgit2 --test ub_safe
...
   Compiling serde_derive v1.0.229
error: building proc macro crate with sanitizers enabled is not supported
error: could not compile `serde_derive` (lib) due to 1 previous error
```

The `asan-ubsan` and `tsan` branches export the sanitizer flag through the global
`RUSTFLAGS`:

```bash
export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=address -Cforce-frame-pointers=yes"
...
exec cargo +nightly "$@" --target $host
```

`RUSTFLAGS` applies to every compilation, host artifacts included. `libgit2`'s
dev-dependency `trybuild` (used by the `compile_fail` suite) pulls `serde`/`toml`,
whose `serde_derive` is a proc-macro; the 2026-10-07 nightly refuses to build a
proc-macro with a sanitizer enabled, so the whole `cargo test` aborts before any test
runs. Because `trybuild` is an unconditional dev-dependency, this blocks *every* test
target, `--lib` included, not just the `compile_fail` suite. The `bsan` branch already
sidesteps `trybuild` (it skips `compile_fail`), but `asan-ubsan` and `tsan` had no
equivalent protection, so both instruments were unrunnable.

(The wave-0 reviews recorded clean ASan/TSan runs for the same `git_error` items, so
this is a regression from either the newer resolved `trybuild` 1.0.122 pulling
`serde_derive`, or the newer nightly tightening the proc-macro rule.)

## Fix

`crustify/scripts/cargo-variant.sh`:

- `asan-ubsan` and `tsan` now pass the sanitizer flag through
  `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS` instead of the global `RUSTFLAGS`.
  Both branches already pass `--target x86_64-unknown-linux-gnu`, so the per-target
  override instruments every crate built for the target (the lib, its tests and their
  dependencies, where libgit2's allocations live, so LSan still sees leaks) while
  leaving host build scripts and proc-macros uninstrumented.
- `tsan` now also skips the `compile_fail` suite for a plain `test` invocation, exactly
  as `bsan` does: `trybuild`'s nested `cargo` cannot build the sanitized dependencies,
  and `compile_fail` is a compile-time check the `plain` variant already runs. The skip
  is injected only when the user passes no `-- <args>` of their own, since the skip and
  the appended `--target` share the one `--` separator (the pre-existing
  "don't pass test filters through it" quirk is unchanged).

### Verification

From a cleaned target directory, through the patched runner:

- `asan-ubsan test -p libgit2 --test ub_safe` — 50 passed (builds, `serde_derive`
  compiles unsanitized, `compile_fail`/trybuild runs under ASan, LSan active).
- `asan-ubsan test -p libgit2 --test equiv` — 20 passed (all `last_*` equiv tests).
- `asan-ubsan test -p libgit2 --lib -- errors` — 27 passed.
- `tsan test -p libgit2 --test ub_safe` — 49 passed, `compile_fail` skipped; the
  threaded `last` tests (`copies_cross_threads_while_they_read`,
  `first_reads_race_to_pin_the_runtime`) report no data race.
- `bsan test -p libgit2 --test ub_safe` — 49 passed (unchanged branch, re-confirmed).

Only the `asan-ubsan` verification needs this fix to build; the `tsan` fix also needs
the `compile_fail` skip. `bsan` was already correct.
