# `open_ondisk` and `add_file_ondisk` transliterated C names

- **items**: `git_config_open_ondisk`, `git_config_add_file_ondisk` (symbols, wave 1
  batch 1, `5a0f495b6`)
- **affected commit**: `d79771f6f`
- **class**: naming convention

## Finding

The conventions ask the safe surface to use ordinary Rust naming: `snake_case` words,
the C name's prefix dropped where the receiver supplies it, and "the idiomatic Rust term
over a transliterated C one". The two wrappers kept libgit2's run-together `ondisk`:

- `Config::open_ondisk(path)` is not even `snake_case` for its two words ("on disk"),
  and the suffix says nothing in Rust. The method opens the file at `path`, which
  `std` spells `File::open(path)`. Its siblings `open_default`, `open_level` and
  `open_global` name what they open.
- `ConfigMut::add_file_ondisk(path, level, repository, force)` adds a file backend.
  "File" already says it lives on disk; libgit2 needs the suffix only to set it apart
  from `git_config_add_backend` in a flat C namespace.

The wave-0 review renamed `parse_int64` and the other parsers for the same reason (see
`../../../wave-0/batch-1/conventions/parse-int-names-transliterate-c`).

## Fix

Renamed `Config::open_ondisk` → `Config::open` and `ConfigMut::add_file_ondisk` →
`ConfigMut::add_file`. The `Wraps:` anchors still name the C functions, so traceability
is unchanged. `open`'s doc now also states the level the file is added at
(`ConfigLevel::LOCAL`, as `git_config_open_ondisk` does). Every caller was updated: the
unit tests, `tests/ub_safe/config.rs`, `tests/equiv/config.rs` and the two
`compile_fail` cases. Their expected `.stderr` is
unchanged, because neither diagnostic quotes the renamed line. No reproducer: the
defect is a name.
