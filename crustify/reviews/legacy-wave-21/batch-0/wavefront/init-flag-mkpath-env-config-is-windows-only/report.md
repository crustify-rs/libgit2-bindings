# `git_repository_init_flag_t`'s record said MKPATH loads global/system config from the environment

- **item**: `git_repository_init_flag_t` (type), wavefront ownership record
- **affected commit**: `5c40c3b08` (libgit2/repository wave 0 batch 8; reviewed at `65285ef7b`)

## Finding

The record's `_comment_agent` said that under MKPATH "repo_init_config passes use_env
to global/system config loading". The only consumer of that `use_env` is
`are_symlinks_supported` (`repository.c:2236`), where the global/system config load is
inside `#ifdef GIT_WIN32` and the parameter is `GIT_UNUSED` otherwise:

```c
#ifdef GIT_WIN32
	if (load_global_config(&config, use_env) < 0 ||
	    git_config_get_bool(&symlinks, config, "core.symlinks") < 0 ||
	    !symlinks)
		goto done;
#else
	GIT_UNUSED(use_env);
#endif
```

Batch 9's `git_repository_open_flag_t` record had this right ("the parameter is
GIT_UNUSED outside GIT_WIN32, so only Windows builds observe it"); the two records
disagreed, and the Rust doc of `InitFlag::Mkpath` followed the wrong one (see
`misc/init-flag-mkpath-env-config-is-windows-only`).

## Fix

Resubmitted `git_repository_init_flag_t` with `--update`: the comment now names the
sole consumer, its `GIT_WIN32` guard, and that on this platform MKPATH makes init read
no configuration from the environment (only `$GIT_COMMON_DIR`, in `lookup_commondir`).
