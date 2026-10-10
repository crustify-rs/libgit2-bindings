# Wavefront: `git_repository_config`'s result was recorded as outliving the repository

- **records**: symbol `git_repository_config` (`ptr_args[0].note`), symbol
  `git_repository_set_config` (`ptr_args[1].note`), type `git_config`
  (`_comment_agent`)
- **submitted by**: libgit2/repository wave 0 batch 2 (`git_config`) and wave 1
  batches 2 and 3 (the routines)
- **defect**: the records said "the config outlives the repository
  (`git_repository_free` disowns it before its own down-ref)", and the wrappers took
  that as licence to return a `ConfigArc` without a borrow and to install any
  `ConfigArc` in any repository.

## Evidence

The owner pointer (`rc.owner`) is cleared by `git_repository_free`, but it is not the
only pointer from the config to the repository:

- `load_config` (`src/libgit2/repository.c:1344`), which `git_repository_config`
  runs through `git_repository_config__weakptr`, adds every configuration file with
  `git_config_add_file_ondisk(cfg, path, level, repo, 0)`;
- the file backend stores it: `config_file_open` (`src/libgit2/config_file.c:104`,
  `b->repo = repo`), uncounted, never cleared;
- it is read on every reparse: `config_file_refresh` (`config_file.c:227`, run by
  `config_file_get` at `:350` and `config_file_iterator` at `:277` when the file
  changed) and `config_file_refresh_from_buffer` (`:207`, run by `config_file_write`
  at `:1209` after every `set` / `delete`) both call
  `config_file_read(.., b->repo, ..)`;
- the parser evaluates `[includeIf "..."]` against it: `parse_conditional_include`
  (`:738`) → `do_match_gitdir` (`:630`, `git_repository__item_path(repo, GITDIR)`,
  reading `repo->gitdir`) or `conditional_match_onbranch` (`:686`,
  `git_repository_refdb(repo)`, which writes `repo->_refdb`).

The ASan trace in
`../../ub_safe/repository-config-keeps-freed-repository-for-includeif/asan-trace.log`
shows the read of the freed repository from `git_repository_path` under
`do_match_gitdir`, reached from `git_repository_set_bare` through a config the freed
repository had lent.

Snapshot backends copy the entries and ignore the repository they are given
(`config_snapshot_open`, `src/libgit2/config_snapshot.c:146`), so
`git_repository_config_snapshot`'s result is bound to no repository; that part of the
records stands.

## Resubmitted

- `git_repository_config.ptr_args[0].note`: the reference borrows the repository
  (`ConfigArc<'r>`); released after it is fine, used after it is not, with the paths
  above.
- `git_repository_set_config.ptr_args[1].note`: the installing repository keeps the
  reference for its whole life and reparses through it, so only a config loaded for no
  repository (`ConfigArc<'static>`) may be installed.
- `git_config._comment_agent`: "may be released after the repository, but not used";
  the `Config<'r>` representation and the contracts of the three routines.
