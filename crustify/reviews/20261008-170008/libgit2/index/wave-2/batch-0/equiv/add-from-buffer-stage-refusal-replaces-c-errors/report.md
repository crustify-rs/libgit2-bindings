# `IndexMut::add_from_buffer` replaced libgit2's own errors for conflict-stage entries

- **item**: `git_index_add_from_buffer`, `crustify/rust/libgit2/libgit2/index/index.rs`
  (`IndexMut::add_from_buffer`)
- **affected**: every call with an entry of stage 1-3 that libgit2 fails before adding
  it: on an index no repository owns (every index the safe API gives an exclusive
  handle to), with a mode other than a file's or a link's, or with more than
  `u32::MAX` bytes
- **affected commit**: `cb77d2e9b` (libgit2/index wave 2 batch 0; reviewed at the
  review-wave base `b9d01cafd`)
- **class**: equivalence, return values and errors. Code and class match (-1,
  `GIT_ERROR_INDEX`); the message is the wrapper's own instead of libgit2's.
- **reproducer**: `src/main.rs` (`LD_LIBRARY_PATH=/target/crustify/builds/plain cargo run`
  from this directory). It exits 101 on the affected commit (`run-affected.log`) and 0
  after the fix (`run-fixed.log`).

## Finding

`git_index_add_from_buffer` (`src/libgit2/index.c:1518`) checks its arguments in this
order: the owner (`"could not initialize index entry. Index is not backed up by an
existing repository."`), the mode (`"invalid filemode"`), the length (`"buffer is too
large"`), then the path (`index_entry_dup`). After those checks it writes the blob and
inserts the entry. A stage 1-3 entry is a use after free only from the insert on: the
new entry is the path's conflict, `git_index__conflict_to_reuc` frees it, and line 1563
reads its path. That needs an owner, a valid mode and a fitting length.

Batch 0 refused every entry whose stage is not 0, before any of libgit2's checks:

```rust
if entry.stage() != Stage::Normal {
    set(ErrorClass::Index, c"cannot add a conflict side from a buffer: give a stage 0 entry");
    return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
}
```

An exclusive handle only ever reaches an index without an owner (see
`IndexRef::has_owner`). So through the safe API every refused call was one libgit2
fails safely, with the owner message. The refusal changed the observable error and
protected nothing. The batch's equivalence test (`add_from_buffer_sequences_match`)
only fed stage-0 entries, so it missed this.

Reproducer output on the affected commit (two independent in-memory indexes):

```
stage 2 file:
  C:    (-1, Some((10, "could not initialize index entry. Index is not backed up by an existing repository.")), 0)
  Rust: (-1, Some((10, "cannot add a conflict side from a buffer: give a stage 0 entry")), 0)
stage 3 tree:
  C:    (-1, Some((10, "could not initialize index entry. Index is not backed up by an existing repository.")), 0)
  Rust: (-1, Some((10, "cannot add a conflict side from a buffer: give a stage 0 entry")), 0)
stage 0 file:
  C:    (-1, Some((10, "could not initialize index entry. Index is not backed up by an existing repository.")), 0)
  Rust: (-1, Some((10, "could not initialize index entry. Index is not backed up by an existing repository.")), 0)

thread 'main' panicked at src/main.rs:86:5:
the safe API's errors differ from libgit2's
```

## Fix

The refusal now applies only when libgit2 would go on to add the entry: the index has an
owner, the mode is `Blob`, `BlobExecutable` or `Link`, and `data` fits in a `u32`. Every
other conflict-stage call reaches libgit2 and fails with its error. libgit2's path check
cannot be run from Rust, so for an owned index the refusal still comes before it. That
case is documented, and an exclusive handle to an owned index is reachable only through
test fixtures.

## Regression

`tests/equiv/index/entries.rs::conflict_stage_buffers_fail_as_c_before_adding` compares
code, class, message and the entries for stages 1-3, step by step. It covers unowned
indexes (file, link, tree mode, `u32::MAX + 1` bytes) and owned ones (submodule and tree
modes, `u32::MAX + 1` bytes). It fails on the affected commit and passes after the fix.
The use-after-free regression
(`index.rs::unit_tests::entries::buffers_refuse_conflict_stages_libgit2_would_free_while_adding`,
owned index, valid modes) still passes unchanged.
