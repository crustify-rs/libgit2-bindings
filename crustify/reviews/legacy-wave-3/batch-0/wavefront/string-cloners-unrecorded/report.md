# The `string` lifetime tier records no cloner

- **item**: `string` (raw-lifetime)
- **affected**: `crustify/wavefront/ownership-store.json`
- **affected commit**: `68cd137c7` (wave 0 batch 0)
- **class**: wavefront findings — incomplete lifecycle discovery

## Finding

On the affected commit `wavefront query symbols --lifetime-for string` returns

```
"dropped_by":    [git_odb_backend_data_free],
"cloned_by":     [],
"constructed_by":[git_odb_backend_data_alloc]
```

The batch recorded the string verdict only on the exported `void` pair's notes.
libgit2 does have string cloners, and the Rust strategy reproduces one of them:
`OdbDataStrFree::dup` / `str_dup` is documented as "`git__strdup` over the exported
routines". `src/util/alloc.c`:

```c
char *git__strdup(const char *str)      /* git__malloc(strlen(str) + 1) + memcpy */
char *git__strndup(const char *str, size_t n)   /* strnlen-bounded */
char *git__substrdup(const char *str, size_t n) /* exactly n bytes + NUL */
```

They are hidden-visibility (not callable from Rust), which is why the strategy
rebuilds `git__strdup` rather than binding it, but the schema's discovery guidance
asks for the string tier's primitives regardless of export — the store already
holds `git__malloc` / `git__free` / `git__calloc` as internal `void` primitives —
and `git__strdup` has over a hundred callers (`git_strarray_copy`,
`git_signature_new`, ...). Without them the tier reads as if libgit2 never copies a
string, and a later run looking for the `git_strarray` / `git_signature` cloners'
underlying primitive finds nothing.

The public API has no string constructor or releaser of its own: no `GIT_EXTERN`
returns an owned `char *` (`git_oid_tostr_s` returns a thread-local static,
`git_oid_tostr` its `out` argument) and none takes a non-const `char *` it frees
(checked over `include/git2`, deprecated items excluded). The typed disposers
`git_strarray_dispose`, `git_buf_dispose` and `git_signature_free` act on struct
arguments and belong to their types' batches.

## Fix

Submitted `git__strdup`, `git__strndup` and `git__substrdup` through
`query symbols --update`: `str` borrowed for the call (`string: true`; `array` set on
the bounded two, whose input need not be terminated), the return an owned, nullable
NUL-terminated string, and `lifetime: {for: "str", is_cloner: {deep: true}}`.
`--lifetime-for string` now lists all three under `cloned_by`.
