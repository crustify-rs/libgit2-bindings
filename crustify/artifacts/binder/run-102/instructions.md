## Wavefront configuration

The committed configurations under `crustify/wavefront/configs/` predate the installed
wavefront's schema, which rejects them (`out_of_scope` is no longer read). Query wavefront
through the campaign-wide configuration converted to the current schema instead:

```bash
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c query ...
```

Its `state_dir` resolves to your tree's `crustify/wavefront/`, so findings you submit with
`--update` land in your tree's ownership store as usual. Leave the committed
configurations as they are: a planner run brings them to the current schema.

## The checkout options' callbacks

With your batch, all three of `git_checkout_options`' callbacks have wrappers:
`git_checkout_progress_cb` landed in wave 330 as `ProgressCallback` (3b654f097), and
your batch wraps the notify and perfdata callbacks. The options' landed accessors
(`set_notify_callback`, `set_progress_callback`, `set_perfdata_callback` and their
getters) still take and return the raw callbacks and payloads, as raw seams waiting
on those wrappers.
