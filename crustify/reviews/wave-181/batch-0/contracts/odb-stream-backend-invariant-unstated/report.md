# `OdbStreamRef::backend` relies on a `backend` invariant that no stated contract asks for

- **objective**: `contracts`
- **where**: `src/libgit2/src/libgit2/odb.rs`, `OdbStreamRef::backend` (`:2647` at
  `9fe83bce6`) and `OdbWritepackRef::backend` (`:3152`), with the unit test
  `unit_tests::stream::stream_getters_read_every_field` (`:4894`)
- **introduced by**: wave 180 batch 0 (`9fe83bce6`), which turned the two getters from raw
  pointers into `Option<OdbBackendRef<'a>>`

## What is wrong

Until `9fe83bce6`, both getters returned the `backend` field as a raw pointer that nothing
dereferenced. They now lend it as an `OdbBackendRef<'a>`, whose safe getters (`version`,
`in_odb`, the `has_<member>` family, `Debug`) read through it. Each getter's `// SAFETY:`
block justifies the lend this way:

```rust
// SAFETY: a non-NULL `backend` is the backend that made the stream (the owner's
// adoption vouches for that), which the odb borrowed for `'o` keeps alive past
// `'a`; the handle only reads it, as the odb's own readers do.
unsafe { OdbBackendRef::from_ptr(backend) }
```

Three paths reach this block. Only the first is covered by the justification:

1. a stream libgit2 made (`OdbRef::open_write_stream`, `open_read_stream`), whose `backend`
   is the live backend of the odb borrowed for `'o`;
2. `OdbStreamBox::from_c`, the owner's adoption. The justification cites it, but the type's
   documentation never asks its caller to vouch for the `backend` field;
3. `OdbStreamRef::from_ptr` / `OdbStreamMut::from_ptr` (ffibox's `CRefSeam` / `CMutSeam`),
   which the comment does not mention. Their `# Safety` section is ffibox's
   `CCell::ref_from_ptr`: "`ptr` must address a live, initialised `Self::C` (or be null) that
   outlives `'a`, and nothing may write it while the handle lives". `OdbStream`'s own
   documentation adds nothing about `backend`.

A caller who honours path 3 exactly, with a live, initialised `git_odb_stream` whose
`backend` is stale, gets a handle whose safe `backend()` lends a dangling `OdbBackendRef`.
The batch's own unit test does this. `stream_getters_read_every_field` adopts a stream with
`backend: ptr::without_provenance_mut(0x1000)` under the comment "`raw` is a live stream in
Rust storage, only read through the handle while it lives", and then calls `s.backend()`.
It only compares the address, so it passes. A `.version()` on the result would read
address `0x1000`. `OdbWritepackRef` has the same gap.

The crate elsewhere reads a handle's `from_ptr` as vouching for the C object's library-level
validity (`commit.rs` cites "`from_ptr`'s caller vouched for" a payload). This finding is
therefore a missing statement, not a soundness hole in the published safe API. No safe
path builds a stream or writepack with a foreign `backend`. The reproducer below shows that
the stated obligation alone does not exclude UB.

## Evidence

`src/main.rs` honours `OdbStreamRef::from_ptr`'s stated obligation: a live, initialised
stream on the stack, outliving the handle and not written meanwhile. Its `backend` is a
`git_odb_backend` that was boxed and then freed. Safe code then calls
`handle.backend().map(|b| b.version())`.

Run from this directory, on a checkout of the affected commit (`d10f34cdd`, the wave tip,
which holds `9fe83bce6`), with the campaign's `.cargo/config.toml` in effect:

```sh
cargo +nightly-2026-10-08 miri run
```

It exits 1:

```text
error: Undefined Behavior: memory access failed: alloc307 has been freed, so this pointer is dangling
    --> <repo>/src/libgit2/src/libgit2/odb.rs:3462:18
     |
3462 |         unsafe { version_ptr.read() }
     |                  ^^^^^^^^^^^^^^^^^^ Undefined Behavior occurred here
     |
     = help: this indicates a bug in the program: it performed an invalid operation, and caused Undefined Behavior
     = help: see https://doc.rust-lang.org/nightly/reference/behavior-considered-undefined.html for further information
help: alloc307 was allocated here:
    --> src/main.rs:17:30
     |
  17 |       let gone = Box::into_raw(Box::new(ffi::git_odb_backend {
     |  ______________________________^
  18 | |         version: ffi::GIT_ODB_BACKEND_VERSION,
  19 | |         ..ffi::git_odb_backend::default()
  20 | |     }));
     | |______^
help: alloc307 was deallocated here:
    --> src/main.rs:22:5
     |
  22 |     drop(unsafe { Box::from_raw(gone) });
     |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
     = note: stack backtrace:
             0: libgit2::libgit2::src::libgit2::odb::OdbBackendRef::<'_>::version
                 at <repo>/src/libgit2/src/libgit2/odb.rs:3462:18: 3462:36
             1: main::{closure#0}
                 at src/main.rs:32:50: 32:67
             2: std::option::Option::<libgit2::libgit2::src::libgit2::odb::OdbBackendRef<'_>>::map::<u32, {closure@src/main.rs:32:40: 32:49}>
                 at /root/.rustup/toolchains/nightly-2026-10-08-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/option.rs:1165:29: 1165:33
             3: main
                 at src/main.rs:32:19: 32:68

note: some details are omitted, run with `MIRIFLAGS=-Zmiri-backtrace=full` for a verbose backtrace
```

BorrowSanitizer reports the same read on the review tree
(`CRUSTIFY_BUILD_VARIANT=bsan cargo bsan run`, exit 1): "error: Undefined Behavior: trying
to access an allocation that has been freed", at `core::ptr::read::<u32>` called from
`OdbBackendRef::version`.

The reproducer reaches no C, so Miri runs it whole. It still fails after the fix, as it
should: the fix states the obligation that this `unsafe` block breaks, and does not change
the getter.

## Fix

- `OdbStream`'s and `OdbWritepack`'s documentation now states the invariant for both raw
  entry points: a stream or writepack adopted with `from_c` or borrowed with a handle's
  `from_ptr` must hold NULL or the live backend that made it, which outlives `'o`, as
  libgit2's do. It names the getter that lends it.
- Both getters' `// SAFETY:` comments now cite the three paths: libgit2 set the field, or
  the caller of `from_c` or `from_ptr` vouched for it, as the type requires.
- `raw_stream`, the unit tests' stream fixture, now leaves `backend` NULL, as for a stream
  no backend set up. This makes the other two tests that adopt it honour the invariant too.
  `stream_getters_read_every_field` points `backend` at a live template backend in Rust
  storage for one pass and leaves it NULL for the other. It asserts both the `Some` path,
  with the backend's address and `version` read through the lent handle, and the `None`
  path. It runs under Miri, where it serves as the regression test of the stated contract.
