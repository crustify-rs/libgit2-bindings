# `UnopenedBackend::open` documented constructors that no longer exist

- **item**: `git_config_backend` (type), `UnopenedBackend::open`,
  `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `df0deef13` (doc from wave 4 batch 0, left stale by the revert
  `c5440d3ee` and the follow-up `a66691e90`)
- **class**: misc (documentation)

## Finding

The doc comment read:

> Opens once, without repository-dependent includes. Memory constructors ignore repo.
> Repository-retaining custom/file constructors are not exposed by this phase type and
> require a lifetime-carrying owner when scheduled.

The memory constructors (`Backend::from_string`/`from_values`) were removed by
`c5440d3ee`. The follow-up then exposed the custom constructor, `UnopenedBackend::new`.
After both changes the comment describes behaviour that no longer exists. It also leaves
out what a caller of this method needs: a `CustomBackend`'s `open` gets `None` for the
repository here, and `ConfigMut::add_backend` is how to lend it one. A store cannot keep
the repository either way (`B: 'static`).

## Fix

The comment now says that `open` gives a `CustomBackend` no repository, that
`ConfigMut::add_backend` lends one, and that repository-retaining *file* constructors are
still unexposed. The behaviour is unchanged. `ub_safe::sys_config::custom_backends::
backend_handles_dispatch_to_the_store` (`open <level> false`) and
`registration_failures_release_the_store` (`open <level> true`) already assert it.
