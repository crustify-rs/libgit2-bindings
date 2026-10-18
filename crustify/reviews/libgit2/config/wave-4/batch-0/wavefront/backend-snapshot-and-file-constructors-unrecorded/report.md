# Two `git_config_backend` constructors had no lifetime record

- **item**: `git_config_backend` (type); symbols `git_config_backend_snapshot`
  (`src/libgit2/config_snapshot.c`) and `git_config_backend_from_file`
  (`src/libgit2/config_file.c`)
- **affected commit**: `e797d4624` (wave 4 batch 0 submitted `from_string`,
  `from_values`, `init_backend` and `git_config_backend_free` only)
- **class**: wavefront

## Finding

`query symbols --lifetime-for git_config_backend` listed three constructors
(`git_config_backend_from_string`, `_from_values`, `git_config_init_backend`). Both
records below had `lifetime: null` and `ptr: null` on every argument:

- `git_config_backend_snapshot(git_config_backend **out, git_config_backend *source)`
  `calloc`s a `config_snapshot_backend`, fills every slot, sets `readonly = 1` and
  writes `*out` only on success. It is the constructor behind the memory backend's and
  the snapshot backend's own `snapshot` slots (`config_mem.c:322`,
  `config_snapshot.c:189`) and `config_file_snapshot`. The Rust wrapper
  `BackendMut::snapshot` adopts its result into a `BackendBox`. `source` is stored
  uncounted and read only by `config_snapshot_open` (`b->source->iterator`), so it has
  to outlive the snapshot's open and no longer. The wrapper relies on exactly this when
  it opens the snapshot while the source is still borrowed.
- `git_config_backend_from_file(git_config_backend **out, const char *path)` allocates a
  writable file backend (two callers: `git_config_add_file_ondisk`, `open_gitmodules`).
  It copies `path`.

`query types --name git_config_backend --lifecycle-ops` showed neither, so a later
batch (wave 5's `git_config_add_backend`, or any reader of `--lifecycle-ops`) would not
learn that a snapshot slot hands out a fresh, unopened owner.

## Correction

Both were submitted with `query symbols --name <sym> --update` (alloc constructor
`for: out`). The records now hold:

- `git_config_backend_snapshot`: `out` is a `by_ref` owned slot written on success
  only; `source` is borrowed (`other`), non-null and not mutated, and must stay live
  through the snapshot's `open`.
- `git_config_backend_from_file`: `out` the same; `path` a borrowed string read during
  the call.

`query symbols --lifetime-for git_config_backend` now lists five constructors and the
one dropper. The store diff adds just these two records, with no reordering.
