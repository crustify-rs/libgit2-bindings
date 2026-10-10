# A repository's config kept reading the repository after it was freed

- **item**: `git_repository_config` (symbol),
  `crustify/rust/libgit2/libgit2/repository/repository.rs`: `RepositoryRef::config`,
  with `git_repository_set_config` (`RepositoryMut::set_config`,
  `repository/sys/repository.rs`) as the second half of the safe path, and the
  `git_config` type (`config/config.rs`: `Config`, `ConfigRef` / `ConfigMut`,
  `ConfigBox` / `ConfigArc`) that carried the wrong lifetime claim
- **affected commit**: `28fd3b00e` (libgit2/repository wave 1 batch 2, which wrapped
  `git_repository_config`); `f591b52ca` (batch 3, `git_repository_set_config`);
  reviewed at `352115fb7`
- **class**: use after free through the safe API (`#![forbid(unsafe_code)]`
  reproducer in `src/main.rs`, ASan trace in `asan-trace.log`)

## Finding

`RepositoryRef::config` returned a `ConfigArc` with no lifetime, on the claim (module
docs of `config/config.rs`, the wrapper's doc, the unit test
`repository_config_outlives_its_repository` and
`ub_safe/config.rs::repository_config_outlives_its_repository`) that "a repository's
config outlives the repository: `git_repository_free` disowns it
(`GIT_REFCOUNT_OWN(config, NULL)`) before releasing its own reference".

The owner pointer is not the only one. `git_repository_config` loads the config through
`load_config` (`repository.c:1344`), which adds every file with
`git_config_add_file_ondisk(cfg, path, level, repo, 0)`, and the file backend keeps that
repository: `config_file_open` (`config_file.c:104`) stores `b->repo = repo`, and every
reparse of the file passes it on (`config_file_read(.., b->repo, ..)` from
`config_file_refresh`, `config_file.c:245`, which `config_file_get` and
`config_file_iterator` run when the file changed; `config_file_refresh_from_buffer`,
`config_file.c:216`, which `config_file_write` runs after every `set` or `delete`). The
parser evaluates each `[includeIf "..."]` section against it
(`parse_conditional_include`, `config_file.c:738`): `gitdir:` / `gitdir/i:` through
`git_repository__item_path(repo, GIT_REPOSITORY_ITEM_GITDIR)` (`do_match_gitdir`,
`config_file.c:630`, reading `repo->gitdir`), `onbranch:` through
`git_repository_refdb(repo)` (`conditional_match_onbranch`, `config_file.c:686`, which
also *writes* the freed repository's `_refdb` slot and makes it the refdb's owner).
`git_repository_free` clears none of these.

So any reparse through a config a freed repository lent reads the freed repository. The
safe API reached it without a config routine wrapper: `RepositoryMut::set_config`
installed the lent config in a second repository, which then wrote through it
(`RepositoryMut::set_bare` sets `core.bare`; `set_workdir(.., true)` sets
`core.worktree`), reparsing the file:

```
ERROR: AddressSanitizer: heap-use-after-free on address 0x70b8e3406ee0 ...
READ of size 8 at 0x70b8e3406ee0 thread T0
    #0 git_repository_path /target/src/libgit2/repository.c:3348:15
    #1 resolved_parent_path /target/src/libgit2/repository.c:3292:13
    #2 git_repository__item_path /target/src/libgit2/repository.c:3323:23
    #3 do_match_gitdir /target/src/libgit2/config_file.c:630:15
    #4 parse_conditional_include /target/src/libgit2/config_file.c:759:16
    #5 read_on_variable /target/src/libgit2/config_file.c:841:12
    #6 git_config_parse /target/src/libgit2/config_parse.c:564:14
    #7 config_file_read_buffer /target/src/libgit2/config_file.c:880:10
    #8 config_file_refresh_from_buffer /target/src/libgit2/config_file.c:216:15
    #9 config_file_write /target/src/libgit2/config_file.c:1209:16
    ... git_config_set_bool <- git_repository_set_bare <- RepositoryMut::set_bare
freed by thread T0 here:
    #1 <RepositoryFree as CDrop<Repository>>::c_drop   (git_repository_free)
    #2 main src/main.rs:45:5
```

The config subsystem's own wrappers (`git_config_get_*`, `git_config_set_*`,
`git_config_iterator_*`), scheduled later, would have reached the same reparse from a
single repository (a `get` after the file changed, any `set`).

Run the reproducer with the ASan prebuild:

```sh
CRUSTIFY_BUILD_VARIANT=asan-ubsan RUSTFLAGS=-Zsanitizer=address \
ASAN_OPTIONS=detect_leaks=1:abort_on_error=1 ASAN_SYMBOLIZER_PATH=/usr/bin/llvm-symbolizer-18 \
LD_LIBRARY_PATH=/target/crustify/builds/asan-ubsan \
cargo +nightly run --target x86_64-unknown-linux-gnu
```

Snapshots are not affected: `git_repository_config_snapshot` builds a config of
snapshot backends, which copy the entries and ignore the repository they are given
(`config_snapshot_open`, `config_snapshot.c:146`), so `config_snapshot` keeps returning
an owner without a borrow.

## Fix

`Config` now carries the borrow of the repository it was loaded for, as `Odb<'r>` and
`Refdb<'r>` do: `Config<'r>`, `ConfigRef<'a, 'r>`, `ConfigMut<'a, 'r>`,
`ConfigBox<'r>` / `ConfigArc<'r>`, hand-written over `CCell` (the `define_ctype!` form
cannot name a lifetime), with `ConfigFree` implementing `CDrop<Config<'r>>` and
`CNew<Config<'static>>`. `Config::new` builds a `ConfigBox<'static>` (loaded for no
repository), `RepositoryRef::config` returns `ConfigArc<'a>` of the repository's
borrow, `config_snapshot` returns `ConfigBox<'static>`, and
`RepositoryMut::set_config` installs only a `ConfigArc<'static>`: the repository keeps
its reference for as long as it lives and reparses the config's files through it, so a
config another repository lent, which that repository may not outlive, is refused at
compile time. The module docs of `config/config.rs` ("The repository a config was
loaded for") and `sys/repository.rs` ("Installed parts") record the contract the config
subsystem's wrappers inherit (`git_config_add_file_ondisk` given a repository binds
the config to it).

Regression evidence: `tests/ub_safe/compile_fail/config_arc_outlives_repository.rs`
(dropping the lent config after the repository, and installing it in another one, both
fail to compile), the `compile_fail` doctests on `Config` (the lifetime only shrinks,
`ConfigMut` is invariant) and `ConfigArc` / `RepositoryRef::config` (no outliving), and
`ub_safe/config.rs::repository_config_reparses_against_its_live_repository`, which
writes through a lent config whose file holds an `includeIf` section while the
repository is live (the reproducer's write, under the borrow). The tests that let a
config outlive its repository (`ub_safe/repository.rs::owned_results_outlive_their_repository`,
`cleanup_keeps_what_was_handed_out`, `ub_safe/config.rs::shared_configs_release_each_reference_once`,
and the unit test, now `repository_config_is_shared_for_the_repository_borrow`) release
it first.
