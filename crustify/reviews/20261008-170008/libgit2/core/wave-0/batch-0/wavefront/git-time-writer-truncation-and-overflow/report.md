# The `git_time` note described the writer's sign rule but not its 32-bit cast and overflow

- **entity**: `git_time` (type record, `_comment_agent`), submitted by libgit2/core
  wave 0 batch 6 (`0181071d8`)
- **class**: incomplete finding — the one C routine that writes the type was
  summarised by its sign rule alone, leaving out two value limits that bound what a
  wrapper may hand it

## Previous finding

> git_signature__writebuf prints '-' when offset < 0 or sign == '-' (the '-0000'
> case), '+' otherwise; git_signature__equal compares the three fields. All-zero is a
> valid value.

## Evidence it is incomplete

`src/libgit2/signature.c:425-442`: the writer prints the seconds as
`(unsigned)sig->when.time` with `%u`, so a `git_time_t` (64-bit) outside `0..=u32::MAX`
is written wrapped — confirmed by `git_commit_create_buffer` writing `author n <e@x>
4294967295 +0000` for `time = -1` and `... 5 +0000` for `time = 2^32 + 5`
(`tests/equiv/types.rs::signature_zones_are_written_from_offset_and_sign`); and it
negates a negative offset as an `int` (`offset = -offset`), which for `INT_MIN` is
signed overflow, undefined behaviour the campaign's UBSan build aborts on. Both are
reachable with values `git_signature_new` accepts and `git_signature__parse` cannot
produce (it caps the zone at ±14:59), so they matter exactly for a Rust `Time` built
by hand and handed to a commit, tag or reflog wrapper.

## Resubmitted

The `_comment_agent` of `git_time` now records the three writing sites (commit, tag,
reflog), the `%02d%02d` zone format, the `(unsigned)` cast and the `INT_MIN` overflow,
and the resulting rule for the wrappers of the writing routines. The field records
(three plain scalars) were verified and left as they were.
