# The perfdata docs promise a report only for a successful checkout

- **objective**: `misc`
- **where**: `src/libgit2/src/libgit2/checkout.rs`: the module's `# Performance data`
  section and `CheckoutOptionsMut::set_perfdata_callback`. Also `git_checkout_perfdata`'s
  `_comment_user` in `crustify/wavefront/ownership-store.json`.
- **introduced by**: wave 330 batch 0 (`eda4ae102`, "wrap git_checkout_perfdata as
  CheckoutPerfdata")

## What is wrong

The batch documents the counts as something only a successful checkout reports:

```rust
//! A checkout that succeeds reports the filesystem calls it made to its performance-data
//! callback as a `git_checkout_perfdata`: ...
```

```rust
    /// ... libgit2 calls it once a checkout succeeds, with counts of the
    /// `mkdir`, `stat` and `chmod` calls it made, which the callback reads by borrowing a
    /// [`CheckoutPerfdataRef`] over its `perfdata` argument for the call.
```

The batch's ownership-store note says the same: "lent to opts.perfdata_cb ... once, after
a successful checkout".

libgit2 does not wait for success. `git_checkout_iterator` calls the callback when it has
written the working directory, and only after that does it write the index, which can
still fail the checkout (`c/src/libgit2/checkout.c:2680-2686`):

```c
	if (data.opts.perfdata_cb)
		data.opts.perfdata_cb(&data.perfdata, data.opts.perfdata_payload);

cleanup:
	if (!error && data.index != NULL &&
		(data.strategy & CHECKOUT_INDEX_DONT_WRITE_MASK) == 0)
		error = git_index_write(data.index);
```

Suppose a consumer trusts the docs and treats a report as proof that the checkout
succeeded, for example by recording the counts as a completed checkout's. That consumer
records a checkout that then returns `GIT_ELOCKED`. Nothing becomes unsound, since the
counts are valid whenever they are lent. The contract the docs state is still wrong.

## Evidence

The regression test added by this review runs a checkout while `.git/index.lock` exists.
The callback is called once, its counts read the same through C and through a
`CheckoutPerfdataRef`, and `git_checkout_tree` then returns `GIT_ELOCKED` (-14):

```sh
cargo test --test equiv perfdata_reads_match_c_on_links_update_only_and_a_failed_index_write
```

Its third step asserts both `rc == ffi::GIT_ELOCKED` and "one report per checkout, failed or
not". It passes on the reviewed code, so the behaviour is libgit2's own. The docs were
what was wrong: there was no defect to reproduce, and no reproducer package was needed.

## Fix

- The module's `# Performance data` section and `set_perfdata_callback` now say that
  libgit2 reports once it has written the working directory and before it writes the
  index, and that a report does not mean the checkout succeeded.
- The ownership-store note was resubmitted through `wavefront ... query types --name
  git_checkout_perfdata --update` with the same correction.
- The equivalence test above stays as the regression test.

Wave 332's `PerfdataCallback` docs ("when it has written everything") were already
accurate. They belong to review wave 333 and are left unchanged.
