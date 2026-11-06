# BorrowSanitizer reports `git_commit_create_v`'s later variadic parents as freed

- **item**: `git_commit_create_v` (`create_from_array`, libgit2/object wave 2 batch 1,
  `e54be84f8`), `crustify/rust/libgit2/libgit2/object/commit.rs`
- **affected tests**: `object::commit::unit_tests::every_array_length_writes_the_slice_commit`
  (lib), `commit::creating::creation_scripts_match` and
  `commit::creating::unwritable_object_databases_fail_alike` (equiv)
- **class**: gate failure. A BorrowSanitizer false positive at the foreign boundary, with no
  UB behind it.

## Finding

The sub-campaign's BSan gate on the wave tip (`26bf1e7a0`) aborted the `lib` binary
12 s in, and failed two `equiv` tests, all with the same report:

```
error: Undefined Behavior: trying to access an allocation that has been freed.
   --> /target/src/libgit2/oid.c:242:2
0: git_oid_cpy
1: validate_tree_and_parents        commit.c:171
2: git_commit__create_internal
3: git_commit_create_v
4: libgit2::libgit2::object::commit::create_from_array::<2>
```

(The equiv runs say "out of bounds for alloc… of size 10b" for the same access.)
`commit_parent_from_varargs` reads each parent with `va_arg` and returns
`git_commit_id(commit)`, a pointer into the commit. The flagged read is that id being
copied.

## Why it is not UB

- Every parent is a `CommitRef` borrowed from a live `CommitArc` for the whole call. The
  commits are alive, the borrow checker holds them, and the same commits go through
  `create` (slice form, one array) cleanly in the same tests.
- ASan/UBSan is clean on all three tests, and on the whole workspace.
- The standalone probe in this directory (`#![forbid(unsafe_code)]`, see `bsan-probe.log`)
  passes one parent, two parents, the *same* live commit twice, and two parents through
  the slice form:

  | call | BSan |
  |---|---|
  | `create_from_array(.., [a])` | clean |
  | `create_from_array(.., [a, b])` | "allocation that has been freed" |
  | `create_from_array(.., [a, a])` | "allocation that has been freed" |
  | `create(.., &[a, b])` | clean |

  The pointer that is valid as the first variadic argument is reported freed as the
  second. So the provenance BorrowSanitizer attaches to a later `va_arg` is stale. These
  arguments travel on the stack: `git_commit_create_v`'s ninth parameter is the count, so
  every parent is past the six integer registers. The result also depends on earlier
  calls: `ub_safe`'s `[p; 16]` passed in the same gate, and the lib unit tests'
  `create_from_array` calls with two parents passed in its rerun.

Only a report that reproduces through the Rust side as a real access to dead or foreign
memory counts as a UB finding. This one does not.

## Change

The three tests are `#[cfg_attr(bsan, ignore = "...")]`, so the gate runs every other
test of their binaries. Every other instrument still runs them. Under BSan the variadic
call is still exercised by `ub_safe::commit::creating::inputs_are_read_for_the_call_only`
(0, 1 and 16 parents) and the lib unit tests, and the creation family's other routines by
the rest of `tests/{ub_safe,equiv}/commit.rs`. `create_from_array` itself is unchanged:
there is no other way to pass a variadic C function its arguments.
