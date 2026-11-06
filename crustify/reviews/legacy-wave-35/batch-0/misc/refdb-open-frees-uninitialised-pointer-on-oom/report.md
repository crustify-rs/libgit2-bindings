# libgit2 frees an uninitialised pointer when it cannot allocate a refdb (`Transaction::new` on OOM)

- **item**: `git_transaction` (type; its constructor `git_transaction_new`),
  `crustify/rust/libgit2/libgit2/refs/transaction.rs`
- **affected**: `Transaction::new`, the first call on a repository whose reference
  database has not been opened yet, when that allocation fails. Every other wrapper that
  opens a repository's refdb lazily is affected the same way.
- **affected commit**: C defect in the target (`src/libgit2/refdb.c:40`); exposed by
  `95f5f77cb` (libgit2/config wave 0 batch 1)
- **class**: misc — a C defect, reachable only through memory exhaustion (or an
  `Alloc` implementation that fails, which its contract allows), documented on the
  wrapper

## Finding

`git_refdb_open` declares `git_refdb *db;` without initialising it and does

```c
	if ((error = git_refdb_new(&db, repo)) < 0)
		goto out;
	...
out:
	if (error)
		git_refdb_free(db);
```

`git_refdb_new` returns through `GIT_ERROR_CHECK_ALLOC(db)` *before* writing `*out`
when `git__calloc` fails, so `git_refdb_free` runs `GIT_REFCOUNT_DEC` on a stack
garbage pointer. `git_transaction_new` reaches it through `git_repository_refdb` →
`git_repository_refdb__weakptr` → `git_refdb_open` whenever the repository has not opened
its refdb yet.

Found while adding coverage for `git_transaction_new`'s refdb failure branch
(`src/libgit2/transaction.c:99`). An allocator that grants the transaction pool's page and
refuses the next allocation (the refdb) crashes the test child:

```
Thread 2 "libgit2::refs::" received signal SIGSEGV, Segmentation fault.
#0  git_refdb_open (out=out@entry=0x7ffff75b17b8, repo=0x7ffff00a7310) at /target/src/libgit2/refdb.c:74
#1  git_repository_refdb__weakptr (out=0x7ffff0112bd8, repo=0x3, repo@entry=0x7ffff00a7310) at /target/src/libgit2/repository.c:1656
#2  git_repository_refdb (out=0x0, repo=0x3, repo@entry=0x7ffff00a7310) at /target/src/libgit2/repository.c:1673
#3  git_transaction_new (out=0x7ffff75b1860, repo=0x7ffff00a7310) at /target/src/libgit2/transaction.c:98
#4  libgit2::libgit2::refs::transaction::Transaction::new (repo=...) at libgit2/libgit2/refs/transaction.rs:157
```

Safe Rust cannot refuse an allocation: installing a failing allocator needs
`unsafe impl Alloc`. So no `#![forbid(unsafe_code)]` program reaches this short of real
memory exhaustion, and the wrapper cannot prevent it, because any refdb access hits the
same code. That is why this is not filed as a UB finding.

## Change

`Transaction::new` now documents the crash. The new inline test
`refs::transaction::unit_tests::out_of_memory::constructors_report_allocation_failure`
refuses only the pool's page, which takes the `git_pool_mallocz` failure branch (covered).
`transaction.c:99` stays uncovered for this reason. The fix belongs in libgit2:
initialise `db` to NULL in `git_refdb_open`.
