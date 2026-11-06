# `git_checkout_notify_cb`'s side notes say when each side is NULL, and get it wrong

- **entity**: `git_checkout_notify_cb` (`include/git2/checkout.h`), parameters `arg2`
  (baseline), `arg3` (target) and `arg4` (workdir)
- **submitted by**: `b694c3653` (wave 332 batch 0, "wrap the checkout notify and perfdata
  callbacks")
- **relied on by**: the same batch's `CheckoutNotification` field docs, which repeat them;
  see `../../../misc/checkout-notify-repeats-and-workdir-side/`
- **resubmitted**: `wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c
  query symbols --name git_checkout_notify_cb --update updates/git_checkout_notify_cb.json`,
  from the repository root, with this directory's `updates/` file

## Previous finding

```sh
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c \
  query symbols --name git_checkout_notify_cb
```

At the wave tip (`9dacc279e`), the three sides' `ptr` blocks were
`{"scalar": {"by_val": true}, "array": null, "string": false, "owned": false,
"borrowed": {"lifetime": "arg:<self>"}, "nullable": true, "mutable": false}`, with these notes:

- `arg2`: "baseline: &delta->old_file of the checkout's diff, lent for the call and only read; NULL for added, ignored, untracked and unreadable deltas and when there is no delta (an untracked or ignored workdir item). Lent as CheckoutNotification::baseline, an Option<DiffFileRef<'call>>."
- `arg3`: "target: &delta->new_file of the checkout's diff, lent for the call and only read; NULL for deleted deltas and when there is no delta. Lent as CheckoutNotification::target, an Option<DiffFileRef<'call>>."
- `arg4`: "workdir: a git_diff_file on checkout_notify's own stack, filled from the workdir index entry (id, path borrowed from the entry, size, mode, VALID_ID), so valid for the call only; NULL when the path is missing on disk. Lent as CheckoutNotification::workdir, an Option<DiffFileRef<'call>>."

## Why it was wrong

The properties are right: every side is borrowed for the call, read only, and may be NULL.
The notes explain when a side is NULL, and they leave cases out:

- `arg4`: "NULL when the path is missing on disk". `checkout_action_with_wd_dir`
  (`c/src/libgit2/checkout.c:612-617`) passes an unmodified delta with a NULL workdir item
  while a directory stands at the path (`checkout_notify(data, GIT_CHECKOUT_NOTIFY_DIRTY,
  delta, NULL)`), then notifies that directory as untracked on its own. The note also calls
  the side a file. `checkout_notify` (`checkout.c:104-117`) fills it from any workdir index
  entry, and a directory's has mode `GIT_FILEMODE_TREE` and a path ending in `/`.
- `arg2` and `arg3`: NULL "when there is no delta (an untracked or ignored workdir item)".
  `checkout_action_wd_only` (`checkout.c:386-415`) also notifies a file the index tracks
  outside the checkout's diff as `GIT_CHECKOUT_NOTIFY_DIRTY`, with no delta, so both sides are
  NULL there too.

The batch's Rust field docs repeated the `arg4` claim: "`None` for a path missing on disk".

Evidence: the unit test `a_directory_in_place_of_a_kept_file_is_notified_more_than_once`
(`cargo test --lib a_directory_in_place_of_a_kept_file_is_notified_more_than_once`) observes
the DIRTY notification with baseline and target sides and no workdir side, and the directory's
`a.txt/` workdir side with mode `0o040000`. The equivalence test
`a_notify_callback_sees_directories_and_ignored_files_as_a_c_callback` sees the same calls
with C callbacks.

## Corrected finding

The same properties, with notes that list every NULL case and that the workdir side can be a
directory. See `updates/git_checkout_notify_cb.json`. Its `ptr` blocks replace the three
parameters' blocks whole. `arg1` and `arg5` are unchanged. The Rust needs only the
documentation fix that the `misc` report describes. `Option<DiffFileRef<'_>>` already admits
every case.
