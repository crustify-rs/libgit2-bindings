# The adapter's Miri regression rebuilt the setup instead of testing it

- **item**: `git_config_backend_foreach_match` (symbol): `BackendMut::foreach_match` and
  its private ABI adapter `PreparedBackendWalk`,
  `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `5fc58a580` (landed by `e532e6312`)
- **class**: internal

## Finding

`foreach_match` puts a `#[repr(C)]` `PreparedBackendWalk` on its stack and hands C a
pointer to its backend base. The adapter's iterator stores a pointer back to the whole
struct, which `next` later reads. The translation batch's validation says the first draft
wrote that pointer through a fresh Rust reborrow, which invalidated the raw tag C keeps
(Miri and BSan caught it). The fix writes it through the same raw pointer:

```rust
let p = ptr::from_mut(&mut prepared);
unsafe { addr_of_mut!((*p).iterator.backend).write(p.cast()) };
```

The regression for that fix, `unit_tests::iterators::backend_walk_adapter_borrowed_bases_and_lending`,
is the only Rust-only test of the adapter, so it is the only one Miri runs. It did not
call any code from `foreach_match`. It built its own `PreparedBackendWalk` and repeated the
fixed lines in its own body. So the code that runs in production was never checked by
Miri. Reintroducing the draft's defect in `foreach_match`, but not in the test, left every
non-BSan suite green on the affected commit:

```rust
let p = ptr::from_mut(&mut prepared);
prepared.iterator.backend = p.cast();   // the draft's place write
```

- `cargo miri test -p libgit2 --lib -- backend_walk`: `backend_walk_adapter_borrowed_bases_and_lending ... ok`
  (the foreign walk test is `ignore`d under Miri)
- plain `lib`, `ub_safe` and `equiv` `backend_walk` tests: all `ok`

Only BSan caught it. BSan runs once per sub-campaign, at the gate in the last wave:

```
help: the accessed tag <680688>(unprotected) later transitioned to Disabled due to a foreign write access at offsets [0x68..0x70]
    --> .../crustify/rust/libgit2/libgit2/config/config.rs:1859:13
1859 | prepared.iterator.backend = p.cast();
  2: <libgit2::libgit2::config::config::PreparedBackendWalk>::next
      at .../crustify/rust/libgit2/libgit2/config/config.rs:1917:51
error: test failed, to rerun pass `-p libgit2 --test ub_safe`
```

(`ub_safe --exact config::backend_walk::backend_walk_patterns_stops_panics_and_independent_reentry`,
BSan, on a snapshot of the affected commit with the line changed.)

## Fix

The setup now lives in one place, `PreparedBackendWalk::run(inner, require_name, walk)`.
It lays the adapter out, initializes the self-pointer through the raw pointer, and lends
`walk` the backend base. After `walk` returns, it hands back the status and the Rust state,
which still owns the real iterator. `foreach_match` calls `run` with a closure that makes
the C call. Its behaviour is unchanged: the state, and so the inner iterator, is still
dropped before `walk_with` resumes a callback panic.

The regression now calls `run` too. Its closure stands in for C: it reads the base's
`iterator` slot, then the handed-out iterator's `next` and `free` slots, and calls them in
C's order. It checks that the iterator is the adapter's own field, and that the lent entry
reads back and is released by the Rust owner only. With the draft's defect put back into
`run`, Miri now fails the test:

```
error: Undefined Behavior: attempting a read access using <832328> at alloc438112[0x68], but that tag does not exist in the borrow stack for this location
    --> libgit2/libgit2/config/config.rs:1932:21
1932 |             let p = addr_of!((*iterator).backend).read().cast::<Self>();
help: <832328> was later invalidated at offsets [0x68..0x70] by a write access
    --> libgit2/libgit2/config/config.rs:1910:9
1910 |         prepared.iterator.backend = p.cast();
```

The fixed tree passes Miri under Stacked Borrows and `-Zmiri-tree-borrows`.
