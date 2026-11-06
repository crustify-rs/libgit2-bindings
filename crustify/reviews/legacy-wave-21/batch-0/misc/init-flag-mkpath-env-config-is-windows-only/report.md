# `InitFlag::Mkpath` claimed init reads the global and system configuration from the environment

- **item**: `git_repository_init_flag_t` (type), `crustify/rust/libgit2/libgit2/repository/repository.rs`,
  `InitFlag::Mkpath`'s doc comment
- **affected commit**: `5c40c3b08` (libgit2/repository wave 0 batch 8; reviewed at `65285ef7b`)
- **class**: documentation states a behaviour the C does not have on this platform; no
  code change, no reproducer

## Finding

The doc said that, because `GIT_REPOSITORY_INIT_MKPATH` shares its bit with
`GIT_REPOSITORY_OPEN_FROM_ENV`, `git_repository_init_ext` with `Mkpath` "reads the
global and system configuration from the environment's paths". `repo_init_config`
(`repository.c:2377`) does derive `use_env` from that bit, but passes it only to
`repo_init_fs_configs` -> `are_symlinks_supported` (`repository.c:2236`), whose
`load_global_config(&config, use_env)` sits inside `#ifdef GIT_WIN32`; every other
build marks the parameter `GIT_UNUSED`. The external-template lookup
(`repo_init_structure`) opens the default config through `git_config_open_default`,
without the flag. So on Linux the only environment `Mkpath` makes init read is
`$GIT_COMMON_DIR` in `lookup_commondir` (which batch 9's `OpenFlag` record already
noted, and `tests/equiv/repository.rs::mkpath_reads_the_environment_as_open_from_env`
tests).

## Fix

The doc now says the symlink probe reads `core.symlinks` from the environment's config
paths on Windows alone and that init reads no configuration from the environment
elsewhere. The wavefront record of `git_repository_init_flag_t` carried the same
claim; see `wavefront/init-flag-mkpath-env-config-is-windows-only`.
