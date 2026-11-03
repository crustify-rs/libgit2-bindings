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
