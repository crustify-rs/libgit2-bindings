# `OdbRef::open_read_stream` hands libgit2 an id of another type than the odb's, which the loose backend formats out of bounds

- **item**: `git_odb_stream` (type), whose batch bound `git_odb_open_rstream` as
  `OdbRef::open_read_stream`, `src/libgit2/src/libgit2/odb.rs`
- **affected commit**: `3f43c106d` (wave 176 batch 1). `b043039e2` (batch 2) added the
  id-type check to every other reader and missed this one. Reviewed at the wave tip
  `1b48ed7f8`.
- **class**: UB reachable from the safe API. It is a heap buffer overflow (AddressSanitizer),
  a write in `git_oid_pathfmt`. In the converse case, part of the path is never written.
- **reproducer**: `src/main.rs` (`#![forbid(unsafe_code)]`; ASan, run as its header says;
  aborts on the affected commit and exits 0 after the fix)

## Finding

`git_odb_open_rstream` (`c/src/libgit2/odb.c:1563`) hands its id to each backend's
`readstream` member under the odb's lock, without comparing the id's type with the odb's
(`db->options.oid_type`). The loose backend's member, `loose_backend__readstream`
(`c/src/libgit2/odb_loose.c:1037`), calls `locate_object` → `object_file_name`
(`odb_loose.c:82`). That function grows the path buffer by `be->oid_hexsize + 2` bytes, the
odb's hex size, but `git_oid_pathfmt` writes as many digits as the *id's* type has. A
SHA-256 id in a SHA-1 odb writes 65 bytes into room for 42. If the objects directory's path
is short enough that the buffer's growth slack cannot absorb the extra bytes (any directory
under `/tmp`), the write runs past the allocation. A SHA-1 id in a SHA-256 odb leaves 23
bytes of the path unwritten, and `git_fs_path_exists` then reads them.

The module docs of `odb.rs` describe exactly this overflow, and batch 2 refuses foreign ids
before every lookup it binds (`OdbRef::fits` / `same_type`: `read`, `read_prefix`,
`read_header`, `exists`, `exists_ext`, `exists_prefix`). `open_read_stream`, which batch 1
bound with the stream type, calls libgit2 with the id unchecked. Its `// SAFETY:` comment
claims only that `id` is "a live `git_oid`, read for the call only". The ownership store's
note for its `oid` parameter says the same ("Read for the call only."), so the obligation
never reached the wrapper. One safe call reaches it:

```rust
let odb = Odb::open(c"/tmp/ors")?;                                 // SHA-1, loose backend
let wide = Oid::from_raw(&[0xab; 32], OidType::Sha256)?;
odb.as_ref().open_read_stream(wide.as_ref());
```

Run from the reproducer's directory, with `<repo>` the repository's root:

```sh
CRUSTIFY_BUILD_VARIANT=asan-ubsan \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zsanitizer=address \
ASAN_OPTIONS=detect_leaks=1:abort_on_error=1 \
LD_LIBRARY_PATH=<repo>/crustify/builds/asan-ubsan \
cargo +nightly-2026-10-08 run --target x86_64-unknown-linux-gnu
```

On the wave tip (`1b48ed7f8`) it aborts with exit status 134:

```
=================================================================
==3684881==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x723ef4212dc0 at pc 0x75def6588572 bp 0x7ffeccab41f0 sp 0x7ffeccab41e8
WRITE of size 1 at 0x723ef4212dc0 thread T0
    #0 0x75def6588571 in git_oid_fmt_substr /target/libgit2-bindings/c/src/libgit2/oid.h:133:10
    #1 0x75def6588756 in git_oid_pathfmt /target/libgit2-bindings/c/src/libgit2/oid.c:153:2
    #2 0x75def657c836 in object_file_name /target/libgit2-bindings/c/src/libgit2/odb_loose.c:94:2
    #3 0x75def657b01e in locate_object /target/libgit2-bindings/c/src/libgit2/odb_loose.c:466:14
    #4 0x75def657b01e in loose_backend__readstream /target/libgit2-bindings/c/src/libgit2/odb_loose.c:1037:6
    #5 0x75def6577f23 in git_odb_open_rstream /target/libgit2-bindings/c/src/libgit2/odb.c:1587:12
    #6 0x5d8c2b5645d9 in <libgit2::libgit2::src::libgit2::odb::OdbRef>::open_read_stream src/libgit2/src/libgit2/odb.rs:2888:13
    #7 0x5d8c2b55fcec in open_read_stream_formats_foreign_id_out_of_bounds::main crustify/reviews/wave-177/batch-0/ub_safe/open-read-stream-formats-foreign-id-out-of-bounds/src/main.rs:44:31
0x723ef4212dc0 is located 0 bytes after 64-byte region [0x723ef4212d80,0x723ef4212dc0)
allocated by thread T0 here:
    #0 0x5d8c2b52c321 in realloc (crustify/tmp/repro-target/x86_64-unknown-linux-gnu/debug/open-read-stream-formats-foreign-id-out-of-bounds+0xf0321) (BuildId: 823af2b6ae2da480008f1ed70fcb6775b8cb325b)
    #1 0x75def643058a in git__realloc /target/libgit2-bindings/c/src/util/alloc.h:29:12
    #2 0x75def643058a in git_str_try_grow /target/libgit2-bindings/c/src/util/str.c:86:12
    #3 0x75def657c7af in object_file_name /target/libgit2-bindings/c/src/libgit2/odb_loose.c:91:6
    #4 0x75def657b01e in locate_object /target/libgit2-bindings/c/src/libgit2/odb_loose.c:466:14
    #5 0x75def657b01e in loose_backend__readstream /target/libgit2-bindings/c/src/libgit2/odb_loose.c:1037:6
    #6 0x75def6577f23 in git_odb_open_rstream /target/libgit2-bindings/c/src/libgit2/odb.c:1587:12
    #7 0x5d8c2b5645d9 in <libgit2::libgit2::src::libgit2::odb::OdbRef>::open_read_stream src/libgit2/src/libgit2/odb.rs:2888:13
    #8 0x5d8c2b55fcec in open_read_stream_formats_foreign_id_out_of_bounds::main crustify/reviews/wave-177/batch-0/ub_safe/open-read-stream-formats-foreign-id-out-of-bounds/src/main.rs:44:31
    ...
SUMMARY: AddressSanitizer: heap-buffer-overflow (crustify/builds/asan-ubsan/libgit2.so.1.9+0x2cb571)
```

The batch's `ub_safe` stream tests opened read streams only for ids of the odb's own type,
and the batch 2 test `ids_of_another_type_are_refused` covers only the readers batch 2
bound, so no test reached this path.

## Fix

`OdbRef::open_read_stream` now refuses an id whose type differs from the odb's, before the
call, through the same `same_type` probe the readers use. It fails with a `StreamError` of
class `GIT_ERROR_INVALID`, code `-1`, and libgit2's wording for the mismatch ("specified
object ID type does not match object database object ID type"). Its docs, its
`// SAFETY:` comment and the module docs' *Reading and writing* section now list it with the
readers.

Regression test: `tests/ub_safe/src/libgit2/odb/stream.rs`,
`read_streams_refuse_ids_of_another_type`. On both id types it opens a read stream for a
foreign id three times, checks the refusal, then streams a blob of the odb's own type. With
the check removed it aborts under ASan with the same `heap-buffer-overflow` in
`git_oid_pathfmt`. With the check, it passes under ASan/UBSan and TSan. After the fix the
reproducer prints the refusal and exits 0, under ASan and under BorrowSanitizer
(`cargo bsan run`, `CRUSTIFY_BUILD_VARIANT=bsan`).

The ownership store's note for `git_odb_open_rstream`'s `oid` now carries the obligation
(see `wavefront/ownership/odb-open-rstream-id-type-unchecked`).
