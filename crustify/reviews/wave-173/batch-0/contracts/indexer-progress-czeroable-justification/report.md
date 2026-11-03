# `CZeroable for IndexerProgress`: the justification cites a reset the indexer does not perform

- **Objective:** `contracts`, the `// SAFETY:` comment of an `unsafe impl`
- **Where:** `src/libgit2/src/libgit2/indexer.rs`, `unsafe impl CZeroable for IndexerProgress {}`,
  as landed by `9f243dd2e` (wave 172, batch 0)
- **Verdict:** the justification rests on a false claim about C. The impl itself is sound, and no
  input breaks it, so there is no reproducer.

## What is wrong

The landed comment read:

```rust
// SAFETY: all zero is every counter at 0, the state the indexer resets a progress to when it
// reads a pack's header: every field is a plain integer, which the getters read and libgit2
// counts from, and there is no teardown.
```

`CZeroable` promises that all-zero is a state that every accessor, every safe wrapper and the
paired teardown accept (ffibox `docs/ownership.md`, *Policies*). The comment supports that with
"the state the indexer resets a progress to when it reads a pack's header". That is false:

- `c/src/libgit2/indexer.c:915-920` (`git_indexer_append`, on the pack header) sets
  `stats->total_objects = total_objects`, the header's object count, which is not 0 for a
  non-empty pack, and zeroes only the five counters that follow it;
- the indexer never writes `received_bytes` (`grep -n received_bytes c/src/libgit2/indexer.c`
  finds nothing). The transports accumulate it: `c/src/libgit2/transports/smart_protocol.c:664`
  and `c/src/libgit2/transports/local.c:506`.

So the all-zero state is not what the indexer resets to. The *callers* zero a progress before
they hand it to libgit2: `git_packbuilder_write` with `git_indexer_progress stats = { 0 };`
(`c/src/libgit2/pack-objects.c:1425`), and `git_smart__download_pack` with `memset(stats, 0, …)`
(`c/src/libgit2/transports/smart_protocol.c:691`).

The obligation still holds for a different reason. Every field is an `unsigned int` or a
`size_t`, so all-zero is a valid value for every getter and setter, and the type has no
teardown. The justification should say that rather than point at a C behaviour that does not
exist. A future reviewer who checked the claim against `indexer.c` would find it false and
could not tell whether the impl is.

## Rerun

```bash
sed -n 913,921p c/src/libgit2/indexer.c
grep -n received_bytes c/src/libgit2/indexer.c   # no output
grep -n "received_bytes +=" c/src/libgit2/transports/*.c
```

## Fix

The comment now justifies the impl by the field types, and cites the callers that zero a
progress:

```rust
// SAFETY: every field is a plain integer, so all zero is every counter at 0, a value every
// getter and setter accepts and libgit2 counts from (its callers zero a progress before handing
// it over: `git_packbuilder_write`, `git_smart__download_pack`), and there is no teardown.
```
