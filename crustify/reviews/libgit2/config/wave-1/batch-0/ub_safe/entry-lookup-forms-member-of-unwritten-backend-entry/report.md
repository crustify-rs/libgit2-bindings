# An entry lookup that rereads an unparsable file forms a member of an unwritten entry pointer

- **item**: `git_config_entry` (type): its safe constructor `ConfigRef::entry`
  (`git_config_get_entry`), `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `512be0487` (libgit2/config wave 1 batch 0); reviewed at `d79771f6f`, and
  reproduced again on the wave tip `1317ac5cb` (after the symbol review) under this report's commit
- **class**: undefined behaviour in C, reached through the safe API
  (`#![forbid(unsafe_code)]` reproducer in `src/main.rs`; UBSan trace in `ubsan-trace.log`)
- **sanitizer**: UndefinedBehaviorSanitizer (the campaign's `asan-ubsan` prebuild)

## Finding

`git_config_get_entry` is `get_entry(out, config, name, true, GET_ALL_ERRORS)`
(`src/libgit2/config.c:829`, `get_entry` at `:780`). It asks each reader, highest priority first:

```c
res = backend->get(backend, key, &be);

if (res != GIT_ENOTFOUND) {
    *out = &be->entry;      /* config.c:811 */
    break;
}
```

`be` is an uninitialised local. Every backend writes it only on success. When a backend's
`get` fails with anything but `GIT_ENOTFOUND`, `get_entry` still evaluates `&be->entry`,
a member access through an indeterminate pointer (C11 6.5.2.3, 6.5.3.2p4). It then returns
the error, and the caller never reads `*out`. A file backend's `get` fails that way
whenever its file changed on disk into something libgit2 cannot parse:
`config_file_get` -> `config_file_refresh` -> `config_file_read` fails, and
`config_file_get` returns at `config_file.c:351`. A failure to take the backend's entry
list (`config_file.c:354`) and the snapshot backend's mutex failure
(`config_snapshot.c:56`) take the same path.

The wrapper read `out` only on success. But the UB happens inside C before the wrapper
sees the code. Under the campaign's UBSan build the stale slot reads NULL and the process
aborts:

```
/target/src/libgit2/config.c:811:16: runtime error: member access within null pointer of type 'git_config_backend_entry' (aka 'struct git_config_backend_entry')
    #0 0x7ee16776ba12 in get_entry /target/src/libgit2/config.c:811:16
    #1 0x5e7d6510db1a in <libgit2::libgit2::config::config::ConfigRef>::entry .../crustify/rust/libgit2/libgit2/config/config.rs:1347:27
    #2 0x5e7d6510a0c1 in entry_lookup_forms_member_of_unwritten_backend_entry::main .../src/main.rs:29:34
    ...
SUMMARY: UndefinedBehaviorSanitizer: undefined-behavior /target/src/libgit2/config.c:811:16
```

The reproducer is safe code only: open a file config (`Config::open`), look a key up, rewrite
the file without a closing `]`, look the key up again. It fails three runs out of three on
the affected commit.

## Reproduce

```sh
cd crustify/reviews/libgit2/config/wave-1/batch-0/ub_safe/entry-lookup-forms-member-of-unwritten-backend-entry
CRUSTIFY_BUILD_VARIANT=asan-ubsan RUSTFLAGS=-Zsanitizer=address \
UBSAN_OPTIONS=print_stacktrace=1:halt_on_error=1 \
LD_LIBRARY_PATH=/target/crustify/builds/asan-ubsan \
cargo +nightly run --target x86_64-unknown-linux-gnu
```

The affected commit exits 1 with the report above. The fixed tree prints
`lookup failed, as in libgit2: failed to parse config file: missing ']' in section header (in …:1)`
and exits 0, under UBSan and under BSan (`CRUSTIFY_BUILD_VARIANT=bsan cargo bsan run`, the
reproducer alone).

## Fix

The wrap campaign does not patch C, so `ConfigRef::entry` no longer calls
`git_config_get_entry` on a config whose backends may reread. It composes the same result
from routines that report a failed reread without forming an entry:

1. `git_config_multivar_iterator_new(name)` normalises the name first, so an invalid name
   fails before any backend, as in C. `git_config_next` then walks every backend,
   lowest priority first. A backend that cannot reread fails while opening its iterator
   (`all_iter_next`, `config.c:469`), returns that error, and the next call moves on to
   the next backend. The highest-priority event decides C's answer. A failure above every
   backend holding the key is C's error, and is returned. A match, or no event at all,
   means C finds the key in the highest backend holding it, or nowhere.
2. Otherwise `git_config_snapshot` copies the config. Its backends never reread, so their
   `get` fails only on a mutex failure. `git_config_get_entry` on the snapshot hands out
   the entry, or C's own not-found error.

A file backend's failed reread is one-shot: `config_file_read` stamps the file before
parsing, so later reads serve the old entries. That is why the walk in step 1 leaves step
2 nothing to fail on. It also gives the documented divergences, now in the wrapper's docs:

- Each lookup copies the configuration. Repeated lookups return distinct entries, so
  `EntryArc::ptr_eq` no longer holds for two lookups of one key. The ub_safe test that
  asserted it now asserts the opposite.
- The walk also rereads backends *below* the key's. A failure there is spent on a lookup
  that succeeds, as C's does, but a later lookup reads that backend's old entries where C
  would report the failure. When this happens the wrapper puts back the thread's previous
  error, which a successful C lookup leaves untouched.

The sys allowlist gains `git_config_iterator_free`, `git_config_multivar_iterator_new`
and `git_config_next`. Wave 4 wraps them as symbols.

## Tests

- `tests/ub_safe/config.rs::entry_and_map_types::rewritten_unparsable_file_fails_lookup_cleanly`:
  the reproducer's scenario. It must not trip a sanitizer, the lookup fails with
  `GIT_ERROR`, the next lookup reads the old entry, and the entry acquired before stays
  valid.
- `config.rs::unit_tests::entry_reports_failed_rereads` covers the corrected behaviour,
  which no C reference can run without the UB. A broken key file fails with libgit2's
  exact parse error, which is also the thread's error, then serves old entries. An invalid
  name fails first without spending the reread. A broken lower file is spent on a
  successful lookup and leaves the thread's error as it was, both seeded and clear.
- `tests/equiv/config.rs::entry_and_map_types::entry_selection_sequences_match_c`
  compares, step by step, only what C defines: two levels, a multivar, a write, a
  snapshot, level and HIGHEST views, and a lower file broken while the key lives above it
  (C never rereads it), then repaired.

## Same C defect outside this workset

`get_entry` also serves `git_config_get_bool`, `_int32`, `_int64`, `_string`,
`_string_buf`, `_path` and `_mapped`. A throwaway probe, the reproducer with
`ConfigRef::get_i32`, aborts the same way, also on the wave tip `1317ac5cb`:

```
/target/src/libgit2/config.c:811:16: runtime error: member access within null pointer of type 'git_config_backend_entry'
    #0 get_entry /target/src/libgit2/config.c:811:16
    #1 git_config_get_int32 /target/src/libgit2/config.c:883:13
    #2 <libgit2::libgit2::config::config::ConfigRef>::get_i32 .../config/config.rs:566:27
```

Those wrappers (`ConfigRef::get_bool`, `get_i32`, `get_i64`, `get_string`,
`get_string_buf`, `get_path`) belong to wave 1 batch 1, which the concurrent symbol review
owns, and `git_config_get_mapped` comes in wave 2. They are not changed here. Each one is
C's `get_entry` followed by a parser over `entry->value`, so each can be rebuilt on
`ConfigRef::entry` followed by the module's existing parser (`parse_bool`, `parse_i32`,
`parse_i64`, `parse_path`).
