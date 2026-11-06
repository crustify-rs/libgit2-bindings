# Miri: both integration suites abort before any Rust-only test runs

- **item**: `void` (raw-lifetime), `crustify/rust/libgit2/libgit2/odb/sys/odb_backend.rs`
- **affected**: `crustify/rust/libgit2/tests/ub_safe/{main,sys_odb_backend}.rs`,
  `crustify/rust/libgit2/tests/equiv/sys_odb_backend.rs`
- **affected commit**: `9640ba589` (wave 0 batch 0)
- **class**: misc (instrument coverage), no UB or equivalence defect

## Finding

The campaign lists Miri as a separate instrument for the Rust-side assumptions
(lifetime, bounds, initialisation, validity, alignment, `repr`). The inline
`unit_tests` module gates every libgit2-calling case with
`#[cfg_attr(miri, ignore = "calls libgit2")]`, so `cargo miri test --lib` runs the
Rust-only cases. The two integration suites do not: each test binary aborts on its
first test, before the Rust-only tests `vec_empty_forms_own_nothing` and
`vec_rejects_oversized_requests_without_panicking` get to run, so Miri produced no
evidence at all for the public API.

`cargo +nightly miri test -p libgit2 --test equiv`:

```
test sys_odb_backend::traced_allocator_sequences ... error: unsupported operation: `readlink` not available when isolation is enabled
    --> .../library/std/src/sys/fs/unix.rs:1967:26
             8: std::env::current_exe
             9: sys_odb_backend::isolated
             10: sys_odb_backend::traced_allocator_sequences
error: aborting due to 1 previous error
error: test failed, to rerun pass `-p libgit2 --test equiv`
```

`cargo +nightly miri test -p libgit2 --test ub_safe`:

```
test compile_fail ... error: unsupported operation: `statx` not available when isolation is enabled
   --> .../library/std/src/sys/pal/unix/weak/syscall.rs:13:26
error: aborting due to 1 previous error
error: test failed, to rerun pass `-p libgit2 --test ub_safe`
```

Disabling isolation does not help, since the strategies call libgit2, which Miri
cannot reach (`MIRIFLAGS=-Zmiri-disable-isolation`):

```
test sys_odb_backend::vec_copies_slices_of_every_size ... error: unsupported operation: can't call foreign function `git_libgit2_init` on OS `linux`
          0: libgit2::libgit2::odb::sys::odb_backend::runtime_pinned::{closure#0}
          9: libgit2::libgit2::odb::sys::odb_backend::data_alloc
          10: libgit2::libgit2::odb::sys::odb_backend::data_dup_len
          15: sys_odb_backend::vec_copies_slices_of_every_size
```

## Fix

Gate every integration test that reaches libgit2 (or runs `trybuild`'s nested cargo)
with `#[cfg_attr(miri, ignore = "...")]`, as the unit tests already do, so the
Rust-only paths run under Miri and the binaries exit successfully. After the fix
`cargo +nightly miri test -p libgit2` passes: 4 unit tests, 2 `ub_safe` tests and the
4 compile-fail doctests run; 20 tests are ignored.

Nothing of the strategies' Rust-side logic is lost to Miri that a test could
exercise: every owner allocates through libgit2, so the only Miri-reachable paths are
the empty-buffer and size-rejection branches, which the retained tests cover.
