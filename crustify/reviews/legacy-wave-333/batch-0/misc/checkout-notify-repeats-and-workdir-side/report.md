# `NotifyCallback` documents one call per path and a workdir side missing only off disk

- **objective**: `misc`
- **where**: `src/libgit2/src/libgit2/checkout.rs`, the module's "Notification callbacks"
  section, `NotifyCallback`'s documentation, and the `path`, `baseline`, `target` and
  `workdir` fields of `CheckoutNotification`
- **introduced by**: wave 332 batch 0 (`b694c3653`, "wrap the checkout notify and perfdata
  callbacks")

## What is wrong

The documentation of the new notification callback states two things about libgit2's calls
that `checkout_notify`'s callers (`c/src/libgit2/checkout.c`) do not honour:

- `NotifyCallback`: "once per conflicting, dirty, updated, untracked or ignored path", and the
  module docs: "once per path in a situation its `notify_flags` select". One path can be
  notified several times. `checkout_action_with_wd_dir` (`checkout.c:612-617`) notifies an
  unmodified delta as `GIT_CHECKOUT_NOTIFY_DIRTY`, then the directory standing at its path as
  `GIT_CHECKOUT_NOTIFY_UNTRACKED`. `checkout_action_wd_only` (`checkout.c:445-448`) notifies
  that directory as untracked again when the workdir iterator reaches it. Under
  `GIT_CHECKOUT_FORCE`, `checkout_action_common` (`checkout.c:279-285`) also notifies the
  file as `GIT_CHECKOUT_NOTIFY_UPDATED`.
- `CheckoutNotification::workdir`: "The file found in the working directory; `None` for a path
  missing on disk." The dirty notification above passes a NULL working-directory side while a
  directory stands at the path (`checkout_notify(data, GIT_CHECKOUT_NOTIFY_DIRTY, delta,
  NULL)`). When the side is present, it can be a directory, with mode `GIT_FILEMODE_TREE` and
  a path ending in `/`, not only a file.

The neighbouring field docs are loose in the same way. `path` never mentions the trailing `/`
of a directory. `target` says "`None` for a path it removes", but every workdir-only
notification (untracked, ignored, and dirty files the index tracks outside the checkout's
diff, `checkout.c:413-415`) has neither a baseline nor a target side, whether or not the
checkout removes anything.

A closure written to these docs breaks on a real checkout. If it keeps one entry per path,
it is surprised by the second untracked call. If it treats `workdir: None` as "nothing on
disk", it decides that a path which a directory blocks is free.

## Evidence

The behaviour is libgit2's, so the callback sees it identically under C and Rust. A
checkout of a repository's index, against that index as the baseline, into a directory where
`a.txt` is replaced by a directory holding one file, and where `x.log` is ignored through
`.git/info/exclude`, notifies (reason, path, [baseline, target, workdir] paths):

```text
safe:   (DIRTY,     "a.txt",  ["a.txt", "a.txt", None    ])
        (UNTRACKED, "a.txt/", [None,    None,    "a.txt/"])
        (UNTRACKED, "a.txt/", [None,    None,    "a.txt/"])
        (IGNORED,   "x.log",  [None,    None,    "x.log" ])
forced: (DIRTY,     "a.txt",  ["a.txt", "a.txt", None    ])
        (UNTRACKED, "a.txt/", [None,    None,    "a.txt/"])
        (UPDATED,   "a.txt",  ["a.txt", "a.txt", "a.txt/"])
        (UNTRACKED, "a.txt/", [None,    None,    "a.txt/"])
        (IGNORED,   "x.log",  [None,    None,    "x.log" ])
```

Both tests below pin this behaviour. The unit test drives the trampoline through
`walk_with` during real `git_checkout_index` runs. The equivalence test runs C callbacks and a
`NotifyCallback` on independent fixtures and compares them call for call:

```sh
cargo test --lib a_directory_in_place_of_a_kept_file_is_notified_more_than_once
cargo test --test equiv a_notify_callback_sees_directories_and_ignored_files_as_a_c_callback
```

Both pass, at the review's fix commit and with the wrapper unchanged. The behaviour was
always there, and only the documentation was wrong. A documentation defect needs no
reproducer.

## Fix

The module docs and `NotifyCallback` now say "for each ... path it meets", that one path can
come more than once, and name the directory-in-place case. The field docs say this:
- `path` carries a trailing `/` for a directory.
- `baseline` and `target` are the sides of a change the checkout plans, both `None` for a
  notification about the working directory alone.
- `workdir` is a file or a directory, and is `None` when nothing is there, and also when a
  directory stands where the baseline and the target keep the same file.

The two tests above were added. They also cover the C paths at `checkout.c:438-448`
(ignored files) and `checkout.c:612-617`, which no test reached before.
