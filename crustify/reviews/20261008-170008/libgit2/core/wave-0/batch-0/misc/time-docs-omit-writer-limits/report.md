# The `Time` docs described libgit2's zone writer but not its truncation and overflow

- **item**: `git_time` (type), `crustify/rust/libgit2/libgit2/core/types.rs`
  (`Time`, `Time::new`, `TimeMut::set_offset_minutes`, module docs)
- **affected commit**: `0181071d8` (libgit2/core wave 0 batch 6; reviewed at `ab5de9104`)
- **class**: documentation gap on a C contract the commit, tag and reflog wrappers
  will have to guard; no Rust change in behaviour

## Finding

The module docs state the rule by which libgit2 writes a signature's zone (negative
when the offset is negative or the sign byte is `'-'`), which is right, but the
writer, `git_signature__writebuf` (`src/libgit2/signature.c:425`, reached from
`git_commit_create*`, `git_tag_create*` and the reflog writer), is narrower than the
`Time` type in two further ways the docs did not mention:

```c
	offset = sig->when.offset;
	sign = (sig->when.offset < 0 || sig->when.sign == '-') ? '-' : '+';
	if (offset < 0)
		offset = -offset;              /* INT_MIN: signed overflow */
	...
	git_str_printf(buf, "%s%s <%s> %u %c%02d%02d\n", ..., (unsigned)sig->when.time, ...);
```

- the seconds are printed through `(unsigned)`, so a `Time` before the epoch or past
  2106 (`seconds()` is an `i64`, which the docs call "negative before it") is written
  wrapped modulo 2^32: `-1` becomes `4294967295`, `u32::MAX + 5` becomes `5`;
- a negative offset is negated as an `int`, so `i32::MIN` — a value `Time::new` and
  `set_offset_minutes` accept, as `git_signature_new` does — is C undefined behaviour
  in the writer; the UBSan build aborts on it.

Neither path was exercised: gcov of the `coverage` build showed
`git_signature__writebuf` at 0/11 lines after the wave's suites.

## Fix

The module docs, `Time::new` and `TimeMut::set_offset_minutes` now record both limits
and that the wrappers of the writing routines must refuse such a time or document the
wrap; the type keeps carrying what libgit2 stores (the C defect is not copied into
Rust, and a `Time` read back from a signature may hold any of these values).

Regression / coverage: `tests/equiv/types.rs::signature_zones_are_written_from_offset_and_sign`
writes a commit buffer (`git_commit_create_buffer`, newly allowlisted in `libgit2-sys`)
for every offset × sign × seconds probe and compares the `author` line with the rule
read off the `Time` handle, including `-0000`, `'+'` over a negative offset, foreign
sign bytes and the wrapped seconds; `i32::MIN` is deliberately left out of the probe,
as the test's comment says. The `git_time` wavefront record was resubmitted with the
same facts (`../../wavefront/git-time-writer-truncation-and-overflow`).
