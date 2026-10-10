# `Odb`'s `Sync` proof omitted the owning repository's object cache

- **item**: `git_odb` (type), `crustify/rust/libgit2/libgit2/odb/odb.rs`: `Odb`,
  `OdbRef` / `OdbMut`, `OdbArc`, `OdbFree`
- **affected commit**: `beff67acb` (libgit2/repository wave 0 batch 4; reviewed at
  `65285ef7b`)
- **class**: wrong `unsafe impl Sync` / `Send` proof. Not reachable through the safe API
  the wave landed (nothing safe gives a repository an odb yet, and no odb routine is
  wrapped on `OdbRef` yet), but the first symbol batch over `git_repository_odb` /
  `git_repository_wrap_odb` / `git_repository_set_odb` and `git_odb_*` would have
  inherited it. Reproducer under ASan: `src/main.rs`, trace in `asan-trace.log`.

## Finding

`Odb`'s proof said shared access from several threads is serialised by libgit2's own
locking: the backends and commit-graph under `db->lock`, the object cache under its
rwlock, the options fixed before publication. That holds for an odb no repository owns.
But `git_repository_set_odb`, `git_repository_wrap_odb` and the repository's lazy
`git_repository_odb` make the repository the odb's *owner* (`GIT_REFCOUNT_OWN`,
`repository.c:99`), and from then on every read and write goes through the owner's
cache: `odb_cache()` (`odb.c:46`) loads `rc.owner` and returns `&owner->objects`,
holding no reference to the repository. `git_repository_free` clears the owner pointer
before its own down-ref, but an odb routine that loaded it just before keeps reading the
repository's cache after `git__free(repo)`.

The wave's `Repository` is `!Send`, which does not help: `OdbRef` is `Send` (from
`Odb: Sync`), so a scoped thread may read through the handle while the owning thread
drops the repository. The reproducer gives a repository the odb the way a safe
`Repository::wrap_odb` would (`git_repository_wrap_odb`, adopted into `RepositoryBox`),
has four threads read through the `Send` handle the way a safe `OdbRef::exists` would
(`git_odb_exists`), and drops repositories on the main thread:

```
ERROR: AddressSanitizer: heap-use-after-free on address 0x726d3ee07a60 ...
READ of size 4 at 0x726d3ee07a60 thread T2
    #0 git_cache_oidmap__idx /target/src/libgit2/cache.c:19:1
    #1 git_cache_oidmap_get /target/src/libgit2/cache.c:19:1
    #2 cache_get /target/src/libgit2/cache.c:145:6
    #3 git_odb_exists_ext /target/src/libgit2/odb.c:791:16
    #4 main::{closure#0}::{closure#0} src/main.rs:49:30
freed by thread T0 here:
    #0 free
    #1 <RepositoryFree as CDrop<Repository>>::c_drop   (git_repository_free)
    #2 main::{closure#0} src/main.rs:54:13
previously allocated by thread T0 here:
    #3 repository_alloc /target/src/libgit2/repository.c:310:25
    #4 git_repository_wrap_odb /target/src/libgit2/repository.c:1284:9
```

Run it with the ASan prebuild:

```sh
CRUSTIFY_BUILD_VARIANT=asan-ubsan RUSTFLAGS=-Zsanitizer=address \
ASAN_OPTIONS=detect_leaks=1:abort_on_error=1 ASAN_SYMBOLIZER_PATH=/usr/bin/llvm-symbolizer-18 \
LD_LIBRARY_PATH=/target/crustify/builds/asan-ubsan \
cargo +nightly run --target x86_64-unknown-linux-gnu
```

The same shape is single-threaded UB too: a second `git_repository_set_odb` /
`wrap_odb` over an odb another repository already owns overwrites the owner, and the
first repository's own lookups then read the second repository's cache after it is
freed. libgit2 checks neither.

## Fix

`Odb` now carries the owning repository's borrow, as `Refdb<'r>` and `Reference<'r>`
already do: `Odb<'r>`, `OdbRef<'a, 'r>`, `OdbMut<'a, 'r>`, `OdbArc<'r>`, hand-written
over `CCell` (the `define_ctype!` form cannot name a lifetime). The constructors build
`OdbArc<'static>`, the type of an odb no repository owns; `OdbFree` also gains `CNew`
for `git_odb_new`, so `OdbArc::new()` builds the same. The `Send` / `Sync` proofs now
name the owner's cache as the one object reached outside the odb, live for `'r`.

The contract the symbol batches inherit (module docs, "The owning repository"):
`git_repository_odb`'s wrapper returns `OdbArc<'r>` of the repository's borrow;
`git_repository_set_odb` and `git_repository_wrap_odb` consume an `OdbArc<'static>`
(the only proof no repository owns the odb) and may hand it back as `OdbArc<'r>`;
`OdbArc::from_c` callers choose `'r` within the owner's life.

Regression evidence: `tests/ub_safe/compile_fail/odb_owner_outlives_repository.rs`
(an `OdbArc<'r>` cannot outlive the repository it borrows) and the doctests on `Odb`
and `OdbArc` (the lifetime only shrinks; `OdbMut` is invariant); the ub_safe fixture's
`Repo::wrap` now consumes the `'static` arc and `Repo::odb` lends `OdbArc<'_>`, and
`ub_safe/odb.rs::a_repository_shares_the_odb_in_either_drop_order` exercises the
orders the type allows. `equiv/odb.rs::a_shared_odb_lives_as_long_as_in_c` keeps the C
fact (the caller's reference outlives the repository) on the C side and records the
Rust divergence.
