# `extensions()` leaks what it copied when a copy fails part-way (C defect)

- **item**: `git_libgit2_opts` (symbol), `crustify/rust/libgit2/libgit2/core/common.rs`
- **affected**: `extensions` (the `GIT_OPT_GET_EXTENSIONS` wrapper)
- **affected commit**: `7fc5171aa` (libgit2/core wave 0 batch 8; reviewed at `ab5de9104`)
- **class**: memory leak inside libgit2 (LeakSanitizer), reachable through the unsafe
  API within its contract; not undefined behaviour, and not repairable from Rust
- **reproducer**: `src/main.rs` (ASan with `detect_leaks=1`; run as described in its
  header; exits non-zero with the leak report below). Its `unsafe` calls honour their
  contracts: the allocator is installed before libgit2 initialises and is a libc family,
  and no other thread runs.

## Finding

`GIT_OPT_GET_EXTENSIONS` is `git_repository__extensions`
(`src/libgit2/repository.c:2088`), which builds its answer in a `git_vector` (an
8-slot array from `git_vector_init`) of `git__strdup` copies of the built-in and user
extensions. When a `git__strdup` or `git_vector_insert` fails it `return -1`s without
`git_vector_dispose_deep`, leaving the array and every copy made so far allocated, and
nothing holds a pointer to them any more. The wrapper reports the error faithfully
(`ErrorClass::NoMemory`) and leaves its own `StrArray` empty, but the C side has
already lost the blocks. On the reproducer (an allocator granting the vector and two
copies, refusing the third):

```
Direct leak of 64 byte(s) in 1 object(s) allocated from:
    #0 realloc
    #1 <…::Counting as libgit2::libgit2::util::sys::alloc::Alloc>::realloc src/main.rs:75:31
    #2 resize_vector /target/src/util/vector.c:38:17
    #3 git_vector_init /target/src/util/vector.c:116:9
    #4 git_repository__extensions /target/src/libgit2/repository.c:2095:6
    #5 git_libgit2_opts /target/src/libgit2/settings.c:399:17
    #6 libgit2::libgit2::core::common::extensions crustify/rust/libgit2/libgit2/core/common.rs:1639:9
Indirect leak of 18 byte(s) in 2 object(s) allocated from:
    #0 malloc
    #2 git_repository__extensions /target/src/libgit2/repository.c:2113:20
    #3 git_libgit2_opts /target/src/libgit2/settings.c:399:17
SUMMARY: AddressSanitizer: 82 byte(s) leaked in 3 allocation(s).
```

The batch's `allocation_failures_report_errors` suite refused *every* allocation, so
`git_vector_init` failed first and nothing was left behind; only a partial refusal
reaches the leak.

## Resolution

No Rust change can release the blocks: they are allocated and dropped inside one C
call. `extensions`'s documentation now states that a copy failing part-way leaks the
names copied until then. No regression test: the leak is the observable, and the
suites run with `detect_leaks=1`, so a test exercising the partial failure would fail
under ASan by design. The defect belongs upstream (`git_repository__extensions` should
`git_vector_dispose_deep(&extensions)` on its error paths).
