# Two equivalence tests write references concurrently, failing the TSan gate on glibc's `tzset`

- **items**: none of this workset; the tests are
  `tests/equiv/types.rs::reference_types_follow_c` (libgit2/core) and
  `tests/equiv/commit.rs::create_options::commits_created_and_amended_with_the_options_match_c`
  (libgit2/object)
- **affected commit**: present on the review-wave base `b83f28c34`; it reproduces
  intermittently
- **class**: misc — test infrastructure; a sanitizer false positive, not a defect in
  the crate

## Finding

`cargo-variant.sh tsan test --workspace` failed the `equiv` binary in two of the three
runs this review made on its tree. The third run, on the base before any change, passed.
Both failures were the same report:

```
WARNING: ThreadSanitizer: data race
  Write of size 8 at 0x72040002a6f0 by thread T418:
    #1 tzset_internal time/./time/tzset.c:401 (libc.so.6+0xe0880)
    #2 current_time /target/src/libgit2/signature.c:172
    #3 git_signature_now /target/src/libgit2/signature.c:186
    #4 git_reference__log_signature /target/src/libgit2/refs.c:472
    ...
    #7 equiv::types::reference_types_follow_c tests/equiv/types.rs:1183
  Previous write of size 8 at 0x72040002a6f0 by thread T38:
    #2 current_time /target/src/libgit2/signature.c:172
    #3 git_signature_now /target/src/libgit2/signature.c:186
    #4 git_signature_default /target/src/libgit2/signature.c:202
    ...
    #8 equiv::commit::create_options::commits_created_and_amended_with_the_options_match_c tests/equiv/commit.rs:3023
```

Every reference write and every default signature reads the time through glibc's
`mktime`, which runs `tzset_internal` and frees and re-`strdup`s the `TZ` copy under a
glibc lock that TSan cannot see. Any two threads doing this get reported. The crate's
suites already run such tests alone in a child on one thread (grep `tzset` in `tests/`),
but these two ran on the harness's shared threads.

## Fix

Each test now runs its body in a child process, using the helpers its file already has:
`signatures::isolated` in `types.rs` (made `pub(super)`) and the top-level `isolated` in
`commit.rs`. The bodies are unchanged, moved into `*_body` functions. The TSan `equiv`
binary then passed three runs in a row (373 tests), as did the whole-workspace TSan run
on the landed tree.
