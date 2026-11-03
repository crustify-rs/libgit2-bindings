# `git_odb_backend`'s lifecycle has no constructors or droppers

- **type**: `git_odb_backend` (`include/git2/sys/odb_backend.h`)
- **submitted by**: wave 180 batch 0 (`9fe83bce6`), which recorded only `git_odb_init_backend`
  (`init`) for the type, and bound `OdbBackendFree` over the type's `free` member
- **resubmitted**: `query symbols --name <routine> --update updates/<routine>.json` for each
  routine below, from the repository root with this directory's `updates/` files, as
  [Corrected lifecycle](#corrected-lifecycle) describes

## Previous lifecycle

```sh
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c \
  query symbols --lifetime-for git_odb_backend
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c \
  query types --name git_odb_backend --lifecycle-ops
```

At the wave tip (`d10f34cdd`), these returned `dropped_by: []` and `cloned_by: []`.
`constructed_by` held only `git_odb_init_backend`. Each routine below had `lifetime: null`
and no submitted record (`_analysis.submitted: false`).

## Why it was incomplete

`docs/type-lifecycle.md` records an `alloc` constructor for every routine that hands the
caller a new object through a `T **` out-parameter. It records a dropper for every routine
that frees the object's own storage. The API publishes both kinds for `git_odb_backend`:

| routine | where | role |
|---|---|---|
| `git_odb_backend_loose` | `c/src/libgit2/odb_loose.c:1234` (via `git_odb__backend_loose`, `:1189`) | `alloc`, `backend_out` |
| `git_odb_backend_pack` | `c/src/libgit2/odb_pack.c:943` | `alloc`, `backend_out` |
| `git_odb_backend_one_pack` | `c/src/libgit2/odb_pack.c:920` | `alloc`, `backend_out` |
| `git_mempack_new` | `c/src/libgit2/odb_mempack.c:193` | `alloc`, `out` |
| `loose_backend__free` | `c/src/libgit2/odb_loose.c:1147`, set as `free` at `:1227` | dropper, `_backend` |
| `pack_backend__free` | `c/src/libgit2/odb_pack.c:841`, set as `free` at `:914` | dropper, `_backend` |
| `impl__free` | `c/src/libgit2/odb_mempack.c:184`, set as `free` at `:207` | dropper, `_backend` |

Each constructor allocates the container its backend is the first member of
(`loose_backend`, `pack_backend`, `memory_packer_db`). It stores the base through the
out-parameter. The caller then owns the backend until `git_odb_add_backend` hands it to an
odb, whose `odb_free` calls the `free` member. The three static routines are the only
implementations of that member in libgit2, and each releases the whole container.
`pack_backend__free` is also called directly by both pack constructors on failure.

The batch identified the roles itself. `OdbBackendBox`'s documentation names the three
public constructors, and `OdbBackendFree` releases through the member. The
type's `_comment_user` says the type is "released only through its own free member", but no
routine record carried the roles. `query symbols --lifetime-for git_odb_backend`, which the
constructor wrappers of waves 442, 464 and 470 and their reviews read, therefore answered
that nothing builds or frees the type.

The Rust is correct: the policy releases through the member, which is the routine every
dropper above is reached through, and the constructors are scheduled with their own
modules. Only the record is incomplete.

## Corrected lifecycle

Each constructor now carries `{"for": "<out>", "is_constructor": {"alloc": true, "init":
false}}`. Its out-parameter gets the block the `alloc` rule reads: `scalar.by_ref` with
the inner pointer `owned`, the outer slot borrowed for the call (`arg:<out>`), `mutable`.
It is not nullable: `git_odb__backend_loose` and `git_mempack_new` reject NULL with
`GIT_ASSERT_ARG`, and the pack constructors write through it unchecked. Each dropper now
carries `{"for": "_backend", "is_dropper": true}`, with `_backend` `owned`, `scalar.by_val`
and `mutable`. `pack_backend__free` and `loose_backend__free` accept NULL (an early return,
a `git__free(NULL)`). `impl__free` does not: `git_mempack_reset` dereferences its argument.
The other parameters (`objects_dir`, `opts`, `idx`) stay pending for their modules.

```sh
wavefront --config /target/libgit2-bindings/crustify/tmp/wf/wavefront-config.json c \
  query symbols --lifetime-for git_odb_backend
```

now lists the three droppers under `dropped_by`, and under `constructed_by` the four `alloc`
constructors besides `git_odb_init_backend`.

The updated records are in `crustify/wavefront/ownership-store.json` in the batch's second
commit, with the fixes.

## Downstream

No Rust changes. When waves 442, 464 and 470 bind the constructors, they should return an
`OdbBackendBox` through the existing `OdbBackendFree` policy. The droppers need no binding
of their own: they are only reached through the `free` member, which the policy calls.
