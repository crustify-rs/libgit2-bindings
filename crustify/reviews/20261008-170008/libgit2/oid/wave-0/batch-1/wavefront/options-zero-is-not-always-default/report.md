# The `git_oid_t` finding over-generalises what an unnamed value means

- **item**: `git_oid_t` (`OidType`, `crustify/rust/libgit2/libgit2/oid/oid.rs`)
- **affected**: `crustify/wavefront/ownership-store.json`, the `_comment_agent` of
  `git_oid_t`
- **affected commit**: `736863259` (wave 0 batch 1)
- **class**: wavefront findings — imprecise and incomplete analysis of the unnamed
  values that flow through the type

## Finding

The submitted note said that `0` "means 'unset, use GIT_OID_DEFAULT (SHA1)' in every
`*_options.oid_type`", listing `diff_generate.c:502` among the sites, and that the
only other unnamed value was `git_oid_type_fromstr`'s result. Three claims do not
hold against the sources:

1. `git_diff_options.oid_type` does not read `0` as `GIT_OID_DEFAULT`. In
   `src/libgit2/diff_generate.c:502` a zero takes the *repository's* type, and a
   non-zero value that differs from it is an error:

   ```c
   if (!diff->base.opts.oid_type) {
           diff->base.opts.oid_type = repo->oid_type;
   } else if (diff->base.opts.oid_type != repo->oid_type) {
           git_error_set(GIT_ERROR_INVALID,
                   "specified object ID type does not match repository object ID type");
           return -1;
   }
   ```

   The header (`include/git2/diff.h:440`) documents the same. A wrapper of
   `git_diff_options` that follows the note and substitutes `OidType::default()`
   for an unset field would hand a SHA1 type to a SHA256 repository and turn an
   "unset" into that `GIT_ERROR_INVALID`.

2. Public routines *return* the unnamed value, not just options structs that take
   it: `git_repository_oid_type(NULL)` returns `0` (`src/libgit2/repository.c:4076`,
   `return repo ? repo->oid_type : 0;`) and `git_remote_oid_type` writes `*out = 0`
   before failing (`src/libgit2/remote.c:1040`). This is the fact that makes the
   fallible `OidType::from_raw` necessary on the *return* side; the note only
   motivated it on the input side.

3. `git_repository_init_ext` validates `opts->oid_type` not at all
   (`src/libgit2/repository.c:2936`: `if (opts->oid_type) oid_type = opts->oid_type;`).
   An unnamed value is written to the new repository's config as
   `git_oid_type_name`'s `"unknown"`, and the function then fails to open the
   repository it just created (`load_objectformat` -> `GIT_EINVALID`, "unknown object
   format 'unknown'"), leaving the directory behind. Observed with the review's
   equivalence test (`tests/equiv/oid.rs::repository_oid_types_follow_the_rust_type`,
   last block), run against the plain build:

   ```
   init_repo(&repo, 3) -> (GIT_EINVALID, 0, (GIT_ERROR_REPOSITORY, "unknown object format 'unknown'"))
   <repo>/config: objectformat = unknown
   ```

   The note's "option-field accessors should map 0 to None" is right but
   incomplete: a typed *setter* is what makes this path unreachable from Rust.

The sites that do default to `GIT_OID_DEFAULT` were verified as listed
(`repository.c:356/364`, `odb.c:307`, `index.c:447`, `commit_graph.c:431`,
`midx.c:537`, `indexer.c:178`, `object.c:633`, plus `diff_parse.c:91`, which the note
did not list); `git_repository_new`, `git_commit_graph_open` and `git_midx_writer_new`
additionally `GIT_ASSERT_ARG` validity after defaulting, which the note did not
record and which matters for the coverage (debug) build, where those asserts abort.

## Fix

Resubmitted the `git_oid_t` note through `query types --name git_oid_t --update`
with the per-site semantics (default vs. repository type vs. asserted), the two
public routines that return `0`, `git_oid_type_fromstr`'s `0`, the
`git_repository_init_ext` quirk, and the guidance that returned values go through
`from_raw` as well. The Rust docs that repeated the over-generalisation are fixed
in `misc/docs-overgeneralise-zero-as-default`.
