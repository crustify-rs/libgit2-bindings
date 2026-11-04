# `tests/ub_safe/fixture.rs` keeps raw stand-ins for wrappers that exist

- **item**: the `ub_safe` suite's fixture module, `tests/ub_safe/fixture.rs`
- **affected commit**: `7cc989d43` (the tip of `crustify/devel` that wave 526 forks from)
- **class**: misc (a test fixture that departs from the testing spec)

## Finding

The suite opts out of its own `unsafe_code` lint only in `fixture.rs`, so that fixture
can stand in for safe constructors the crate did not have yet. The testing spec says
each such fixture is replaced once its safe constructor exists. On the affected commit,
the fixture calls 51 distinct `ffi::` functions. 23 of them now have safe or unsafe
wrappers in `src/`, yet the fixture still calls them raw, in 117 `unsafe` blocks.
Several of its docs still say "stands in for … (a later run), which replaces it" about
wrappers that have since landed: `git_object_type` (`ObjectRef::kind`), the odb read
(`OdbRef::read`) and the `git_libgit2_opts` setters (`settings::set_user_agent`,
`settings::set_template_path`).

Evidence (run from the repository root):

```bash
git show 7cc989d43:tests/ub_safe/fixture.rs | grep -o 'ffi::git_[a-z0-9_]*(' | sort -u | wc -l   # 51
git show 7cc989d43:tests/ub_safe/fixture.rs | grep -c 'unsafe {'                                 # 117
grep -rn '#\[doc(alias = "git_odb_read")\]' src   # and likewise for each function below
```

No reproducer: the finding is the fixture's conformance, not a behaviour.

## Fix

Every fixture use whose raw calls the safe API covers now goes through that API. No
test changed, nor what any test checks, and `src/` is unchanged. Each fixture keeps its
signature, so no test file changed either. After the fix the fixture calls 34 distinct
`ffi::` functions in 92 `unsafe` blocks.

### Replaced

| fixture | raw calls replaced | now through |
|---|---|---|
| `filter_list`, `stream_buffer` (and its priming loop) | `git_filter_list_new`, `git_filter_list_free` | `FilterList::new` over a `Repository::new()` repository, released on drop |
| `refused_stream` (the refused filter's callback) | `git_error_set_str` | `errors::set_str(ErrorClass::Filter, …)` |
| `set_str_option` | `git_libgit2_init`, `git_libgit2_opts` (`SET_USER_AGENT`, `SET_TEMPLATE_PATH`) | the fixture's `init()`, `settings::set_user_agent` / `settings::set_template_path` (unsafe: process-global, callers run isolated) |
| `create_ext::create` | `git_oid_from_string`, `git_repository_odb`, `git_odb_read`, `git_odb_object_data`, `git_odb_object_size`, `git_odb_object_free`, `git_odb_free` | `RepositoryRef::odb`, `OdbRef::read`, `OdbObjectRef::data` |
| `create_ext::has_reference` | `git_reference_free` | adoption into a `ReferenceBox`, released on drop |
| `annotated_commit::hex` | `git_oid_tostr` | `OidRef::to_hex_string`, over the new safe `oid_of` copy |
| `annotated_commit::CommitRepo::new` | `git_repository_odb`, `git_odb_write`, `git_odb_free` | `RepositoryRef::odb`, `OdbRef::write`, and the new safe `raw_oid` copy for the raw ids its tests read |
| `worktree_add::AddRepo::new` | `git_repository_odb`, `git_odb_write`, `git_odb_free` | `RepositoryRef::odb`, `OdbRef::write`, `OidRef::to_hex_string` |
| `object_and_tree_entry::object_view` | `git_object_type` | `ObjectRef::kind` |
| `object_and_tree_entry::write_tree` | `git_blob_create_from_buffer` | `blob::create_from_buffer` |
| `object_and_tree_entry::write_raw` | `git_odb_write` | `OdbRef::write` |
| `apply` | `git_buf_dispose` (and the raw slice read) | adoption with `Buf::from_raw`, released on drop |
| `filter_probe::hex` | `git_oid_tostr` | `OidRef::to_hex_string`, over `oid_of` |

`filter_list` used to build its list with no repository, since none of its filters read
one. `FilterList::new` requires a repository, so the list now borrows an empty
`Repository::new()` repository, which the ident clean filter and the test filters never
read. The suite's assertions over the streamed bytes are unchanged and pass.

### Remaining, with the reason

Six of the 23 functions are still called raw, because their wrappers do not cover the
form the fixture needs. Each fixture's docs say why:

| fixture | raw call | why the wrapper does not cover it |
|---|---|---|
| `blob_stream` | `git_blob_create_from_stream` | `blob::create_from_stream` returns a `BlobWriter`, which never lends its stream as a `WriteStreamBox`. The `git_writestream` tests over a C stream need one. |
| `commit_blob` | `git_blob_create_from_stream_commit` | `BlobWriter::commit` commits only a writer, not a stream adopted as a `WriteStreamBox` (the same reason; docs added). |
| `fill_from_option` | `git_libgit2_opts` (`GET_USER_AGENT`, `GET_TEMPLATE_PATH`) | `settings::user_agent` and `settings::template_path` return a fresh `CString`. The `buf` tests check how a `Buf` survives libgit2 reusing and replacing the caller's allocation (docs reworded). |
| `stage::stage` | `git_index_read_tree`, `git_index_add` | `IndexMut::read_tree` and `IndexMut::add` take an exclusive handle, which no safe wrapper hands out for a repository's shared `IndexArc`. |
| `index::add_staged` | `git_index_add` | the same: it takes either owner, and the shared one never yields an `IndexMut`. |
| `index::repository_index_box` | `git_repository_index` | it adopts one more counted reference as an `IndexBox`, for the exclusive handle no wrapper gives over a repository's index; `RepositoryRef::index` returns an `IndexArc`. |

The fixture's other 28 FFI functions have no safe counterpart yet, among them
`git_reference_lookup`, `git_filter_list_push`, `git_filter_list_stream_buffer`,
`git_filter_list_apply_to_buffer`, `git_filter_list_length`, `git_filter_register`,
`git_annotated_commit_id` / `_ref`, the `git_remote_*` calls and the submodule calls. They
stay as they were. The fixture remains an open departure from the `ub_safe` contract
until those land.

## Validation

On the fixed tree (`CRUSTIFY_BUILD_VARIANT=plain` unless named):

- `cargo test --test ub_safe`: 805 passed. `compile_fail` fails only on the five snapshots
  that fail on this host before any change (`commit_arc_no_mut_handle`,
  `mailmap_not_clone`, `object_arc_no_mut_handle`, `tag_and_tree_arc_no_mut_handle`,
  `tree_builder_not_clone`: the installed ffibox 0.1.1's path where the snapshots expect
  `$FFIBOX`), exactly as on the affected commit.
- `crustify/scripts/cargo-variant.sh asan-ubsan test --test ub_safe -- --skip compile_fail`:
  805 passed.
- `crustify/scripts/cargo-variant.sh tsan test --test ub_safe -- --skip compile_fail`:
  805 passed.
- `cargo +nightly-2026-10-08 miri test --test ub_safe -- --skip compile_fail`: 184 passed, 615
  ignored (they call libgit2), no Miri error.
- `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo +nightly-2026-10-08 fmt --all --check`: clean.
- `cargo test --workspace --no-fail-fast`: every binary and the doctests pass but for the
  same five snapshots.
