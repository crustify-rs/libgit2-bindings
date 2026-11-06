# `tag::foreach` panics where `git_tag_foreach` returns 0: a closure breaking with `GIT_ITEROVER`

- **item**: `git_tag_foreach` (symbol), `crustify/rust/libgit2/libgit2/object/tag.rs`
- **affected**: `tag::foreach`, for a `ForeachCallback` whose closure returns
  `ControlFlow::Break(-31)` (`GIT_ITEROVER`)
- **affected commit**: `06cf4d560` (libgit2/object wave 2 batch 3; reviewed at the
  review-wave base `d6a6d1f95`)
- **class**: equivalence — the return value: C returns 0 after the callback stopped the
  walk, the safe wrapper panics (`debug_assert_eq!`) in every build with debug
  assertions, `cargo test` and `cargo run` included
- **reproducer**: `src/main.rs` (`cargo run` from this directory; exits 101 on the
  affected commit, 0 after the fix)

## Finding

`git_tag_foreach` (`src/libgit2/tag.c:496`) walks every reference through
`git_reference_foreach_name` with `tags_cb`, which hands each `refs/tags/*` name and its
resolved id to the user callback and returns whatever the callback returned.
`git_reference_foreach_name` (`src/libgit2/refs.c:712`) stops at the first non-zero
answer, and then:

```c
	if (error == GIT_ITEROVER)
		error = 0;
```

That line exists for the iterator's own end (`git_reference_next_name` returns
`GIT_ITEROVER`), but it also swallows a callback that stopped the walk with -31: C's
caller gets 0, as for a finished walk.

`tag::foreach` records the code the closure broke with in `ForeachCallback::stopped`
(that is how it tells the closure's codes from libgit2's own errors) and then asserted
that libgit2 returned it:

```rust
if let Some(code) = callback.stopped {
    // libgit2 stops at the first non-zero answer, and returns it.
    debug_assert_eq!(rc, code.get());
    return Ok(ControlFlow::Break(code));
}
```

For a break with -31 `rc` is 0, so the assertion fails inside the safe call. The closure
is ordinary safe code: `ControlFlow::Break(NonZero::new(ffi::GIT_ITEROVER).unwrap())`,
or any `ErrorCode::IterOver.raw()` a closure propagates from another libgit2 call it
made. The wavefront record of `git_tag_foreach` stated the same wrong rule ("a non-zero
return stops the walk and is returned"; see `wavefront/tag-foreach-iterover-return-unrecorded`).

Equivalence comparison, both sides on `testrepo.git`, both callbacks breaking with -31 at
the first tag (`src/main.rs`, affected commit):

```
C:    rc 0, saw ["refs/tags/annotated_tag_to_blob"], error "could not find '/work/.gitconfig' to stat: No such file or directory"

thread 'main' panicked at crustify/rust/libgit2/libgit2/object/tag.rs:1084:9:
assertion `left == right` failed
  left: 0
 right: -31
Rust panicked where C returned 0 after visiting ["refs/tags/annotated_tag_to_blob"]
```

A release build returned `Ok(Break(-31))`, so the defect is the panic, and the
disagreement between build profiles.

## Fix

`foreach` keeps reporting the closure's break, `Ok(ControlFlow::Break(-31))`: the walk did
stop early, and `Continue` is documented as "every tag was visited". The assertion now
admits libgit2's 0 for -31 only, and the docs state the divergence (C cannot tell such a
walk from a finished one) and correct the error clause: libgit2 sets its "callback
returned" error only when no error is set yet, whoever set it (the reproducer shows a
stale configuration error surviving the walk on both sides).

After the fix:

```
C:    rc 0, saw ["refs/tags/annotated_tag_to_blob"], error "could not find '/work/.gitconfig' to stat: No such file or directory"
Rust: Ok(Break(-31)), saw ["refs/tags/annotated_tag_to_blob"], error "could not find '/work/.gitconfig' to stat: No such file or directory"
```

## Regression tests

- `tests/equiv/tag.rs::repository_tags::iterover_breaks_match_c_but_for_the_code`: the
  reproducer as a suite test. It compares the tags visited and the thread's error, and
  pins the documented divergence (C 0, Rust `Break(-31)`). It panics at
  `tag.rs:1089` with the old assertion restored.
- `object/tag.rs::unit_tests::foreach_tells_breaks_from_failures`: `GIT_ITEROVER` added
  to the codes a closure breaks with (the corrected behaviour). It fails the same way with
  the old assertion.

No other callback wrapper in the workset routes a closure's code through
`git_reference_foreach_name`. `git_tree_walk` returns the callback's negative code
unchanged, `WalkAction` and `TreeBuilderFilter` answers are fixed values, and
`CreateCallback`/`ParentCallback` never reach a reference iterator.
