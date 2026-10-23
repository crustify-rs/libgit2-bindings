# BorrowSanitizer reports the `end` pointer of glibc's `strtoul` as out of bounds in libgit2's date parser

- **item**: `git_signature_default_from_env` (`Signature::default_from_env` and its author
  and committer forms, libgit2/object wave 2 batch 3, `06cf4d560`),
  `crustify/rust/libgit2/libgit2/object/signature.rs`
- **affected test**: `signature::identity_and_dates_from_the_environment` (equiv)
- **class**: gate failure. A BorrowSanitizer false positive at the libc boundary, with no
  UB behind it.

## Finding

The BSan gate on the wave tip failed the test. Run alone, it reports from the C reference
side of the comparison:

```
error: Undefined Behavior: an access of size 1b at offset 0xfffff4200ca486a4 is out of bounds for alloc12416 of size 18b.
0: match_digit
1: user_from_env
2: git_signature_default_from_env
3: equiv::signature::c_calls
```

The safe API alone reaches it too. The probe in this directory (`#![forbid(unsafe_code)]`,
`bsan-probe.log`) calls `Signature::default_committer_from_env` with
`GIT_COMMITTER_DATE=2005-04-07T22:13:13-0700` and gets the same report at
`src/util/date.c:347`, `switch (*end)`. With `GIT_COMMITTER_DATE=1700000000 -0130` it is
clean.

## Why it is not UB

`match_digit` (`src/util/date.c:323-350`) runs `num = strtoul(date, &end, 10)` and then
reads `*end`. glibc is not instrumented, so the `end` it stores carries no provenance
BorrowSanitizer recognises. BSan checks it against whatever allocation its shadow last
held (an 18- or 190-byte block, at an offset of about -2^44: not an address the process
could read at all), while the real pointer lies inside `date`, the `git_str` that
`git__getenv` filled. An epoch date of more than eight digits returns before `*end`
(`if (num >= 100000000 && nodate(tm))`), which is why the `ub_safe` signature tests
(epoch dates) pass the same gate. ASan/UBSan is clean on the test and on the whole
workspace.

## Change

The test is `#[cfg_attr(bsan, ignore = "...")]`. Every other instrument still runs it, and
`tests/ub_safe/signature.rs` keeps the environment paths under BSan with epoch dates.
Nothing in the wrapper can avoid the report: the parse happens inside libgit2 for any
date `git__date_parse` takes digit by digit.
