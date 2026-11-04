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

## `git_checkout_options` is already bound

`CheckoutOptions` landed in October, before its callbacks had wrappers, so its notify,
progress and perfdata accessors are raw seams: `set_notify_callback`,
`set_progress_callback` and `set_perfdata_callback` are `unsafe fn`s taking the C callback
and a `*mut c_void` payload, and `notify_callback`, `progress_callback` and
`perfdata_callback` return them. Waves 330 and 332 since wrapped the three callbacks as
`ProgressCallback`, `NotifyCallback` and `PerfdataCallback` (3b654f097, b694c3653), but left
the accessors raw for the checkout routines' wrappers (wave 356), which are not yet bound.

Your batch re-binds the type with those wrappers: the callback setters become safe and
take the wrappers, and the getters return no raw pointer. The rest of the type's surface
stays as landed unless your analysis finds it wrong. Points the current design leaves open:

- the options already borrow for `'o`, are invariant in it through `CheckoutOptionsMut`,
  are neither `Send` nor `Sync` and are not `Clone`, which a callback borrowed for `'o`
  can rely on;
- a closure's panic is resumed today when `walk_with` returns; a callback stored in the
  options needs another point where it resumes, which the checkout routines' wrappers
  will reach;
- options C filled in, and options embedded in `git_worktree_add_options` (`worktree.rs`
  reaches them as `CheckoutOptionsRef`/`CheckoutOptionsMut`), may hold callbacks no Rust
  wrapper stored;
- any raw callback installation C interop still needs stays on the seam, not the public
  API.

Update the landed tests that go through the raw accessors, including the compile-fail case
`checkout_options_raw_seams_require_unsafe`, and the crate's own callers.
