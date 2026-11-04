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

## Split review (departure)

Review wave 331 reviews wave 330 in two batches, one per kind, at the user's request:
this batch reviews only the commits in your task. Wave 332 landed after wave 330
(b694c3653) before this review ran; it is reviewed by review wave 333, beside you.
