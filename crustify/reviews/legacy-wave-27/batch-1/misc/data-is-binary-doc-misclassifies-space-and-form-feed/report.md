# `data_is_binary`'s doc counted whitespace as neither printable nor non-printable, but C counts space and form feed as printable

- **item**: `git_blob_data_is_binary` (symbol), wrapped as
  `crustify/rust/libgit2/libgit2/object/blob.rs::data_is_binary` (libgit2/object wave 0
  batch 6, `d7cf8bff1`)
- **affected commit**: `6583ab4ad` (the review wave's base)
- **class**: documentation defect; the wrapper's verdicts are right (every probe of
  `tests/equiv/blob.rs` agrees with C), only the contract text was wrong

## Finding

The wrapper's doc described the heuristic as

> Printable bytes are those above `0x1f` but DEL, and backspace, escape and form feed;
> whitespace counts as neither.

`git_str_is_binary` (`src/util/str.c:1270-1275`) classifies a byte in this order:

```c
if ((c > 0x1F && c != 127) || c == '\b' || c == '\033' || c == '\014')
	printable++;
else if (c == '\0')
	return true;
else if (!git__isspace(c))
	nonprintable++;
```

Space (`0x20`) is above `0x1f`, and form feed (`0x0c`) is named by the first clause,
so both are counted as printable before `git__isspace` is consulted; only tab, line
feed, vertical tab and carriage return count as neither. The sentence therefore
contradicted itself for the two whitespace bytes it mentioned, and a consumer taking
it at its word predicts the wrong verdict: 128 spaces followed by one `0x01` byte is
text by C's count (`128 >> 7 = 1`, not below one non-printable byte), where "whitespace
counts as neither" predicts binary (`0 < 1`). 128 tabs followed by `0x01` is binary.

Observed against the `plain` prebuild through `git_blob_data_is_binary` (the
`tests/equiv/blob.rs::whitespace_classes` probe added here):

| input | C | the old doc's prediction |
|---|---|---|
| 128 × `' '`, then `0x01` | text (`0`) | binary |
| 128 × `0x0c`, then `0x01` | text (`0`) | binary |
| 128 × `'\t'`, then `0x01` | binary (`1`) | binary |
| 128 × `'\n'`, `0x0b` or `'\r'`, then `0x01` | binary (`1`) | binary |

The Rust fallback for oversized slices (`wide::is_binary`) already mirrored C exactly,
so no verdict changes.

## Fix

The doc now reads: printable bytes are those above `0x1f` but DEL (space included),
and backspace, escape and form feed; tab, line feed, vertical tab and carriage return
count as neither; every other byte below `0x20`, and DEL, is non-printable.

Regressions: `blob.rs::unit_tests::whitespace_classes` asserts each class through
`wide::is_binary` (under Miri too) and through `data_is_binary`; the C corpus of
`wide_heuristic_matches_libgit2` gains a 128-byte run of each whitespace and control
byte; `tests/equiv/blob.rs::whitespace_classes` compares C and Rust on runs of 127 and
128 of each such byte, and pins C's own verdicts for the space and tab runs.
