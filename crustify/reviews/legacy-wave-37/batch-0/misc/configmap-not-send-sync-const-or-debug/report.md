# `ConfigMap` cannot form a `static` mapping table: not `Sync`, `new` not `const`, no `Debug`

- **item**: `git_configmap` (type), `crustify/rust/libgit2/libgit2/config/config.rs`
- **affected commit**: `512be0487` (libgit2/config wave 1 batch 0)
- **class**: misc (API ergonomics). The existing surface is sound.

## Finding

`ConfigMap<'s>` holds a `git_configmap_t`, an `int` and a `str_match` pointer. The
pointer is NULL or the pointer of a `&'s CStr` that `ConfigMap::new`, `set_match` or
`set_str_match` stored. The record has no interior mutability, no thread affinity and
nothing to release. C only reads mapping records (`git_config_lookup_map_value`,
`git_config_get_mapped`, `git_config_lookup_map_enum`) and never retains them. Still:

- Its `PhantomData<(*const (), &'s CStr)>` marker made the record and its
  `ConfigMapRef`/`ConfigMapMut` handles (`CBorrowedPtr`) `!Send` and `!Sync`.
- `ConfigMap::new` and `MapMatch::map_type` were not `const fn`.

So the way C declares these tables, a `static` array shared by every thread, could not be
written in Rust. The header's own example (`git2/config.h`, `autocrlf_mapping[]`) and
libgit2's `config_cache.c` / `submodule.c` tables are all static. A Rust caller had to
rebuild the table on every lookup and keep it on the calling thread. No record could be
handed to another thread either. Wave 2 wraps `git_config_get_mapped` and
`git_config_lookup_map_value`, whose natural argument is `&[ConfigMap<'_>]`, often a
`static`.

The record and its handles also had no `Debug`, so `{:?}` on a record, or a derived
`Debug` on a struct holding one, did not compile.

This is the same gap the wave-0 review fixed on the sibling borrowed-string record
`BackendMemoryOptions<'o>`
(`../../../wave-0/batch-0/misc/memory-options-not-send-sync-or-debug`). `TreeUpdate<'p>` and
`FilterOptions` are `Send + Sync` under the same argument.

Before the fix, this did not compile (E0277, `*const ()` cannot be shared between threads;
E0015, non-const fn in a static):

```rust
static AUTOCRLF: [ConfigMap<'static>; 3] = [
    ConfigMap::new(MapMatch::False, 0),
    ConfigMap::new(MapMatch::True, 1),
    ConfigMap::new(MapMatch::String(c"input"), 2),
];
```

## Fix

- `MapMatch::map_type` and `ConfigMap::new` are `const fn`.
- `unsafe impl Send/Sync for ConfigMap<'_>`, `Send + Sync` for `ConfigMapRef` (as `&T`),
  `Send` for `ConfigMapMut` (as `&mut T`), and `Sync` for it (a `&ConfigMapMut` reaches
  only the getters). Each carries a `// SAFETY:` comment with the same argument as
  `BackendMemoryOptions`: the string pointer is a `&CStr` borrow, which is `Send + Sync`,
  and every write goes through the exclusive handle. `'s` still ties the record to its
  string, so a record holding a non-`'static` borrow cannot reach `thread::spawn`.
- `Debug` for the record and both handles prints `map_type`, `str_match` and `map_value`.
- The type docs gain a runnable `static AUTOCRLF` example and a `compile_fail,E0597`
  doctest that sends a record borrowing a local `CString` to `thread::spawn`. Before the
  fix that doctest failed with E0277 instead, so it also pins the `Send` impl.

## Tests

- `tests/ub_safe/config.rs::entry_and_map_types::maps_cross_threads` (TSan, Miri): four
  threads read a `static` table at once; copied `ConfigMapRef` handles and `&ConfigMap`
  read on scoped threads; a `ConfigMapMut` writes on another scoped thread; a `Vec` of
  `'static` records moves to a detached thread.
- `config.rs::unit_tests::debug_and_thread_traits` (Miri) checks the `Send + Sync` bounds
  of the record and both handles, a `const` table, and the `Debug` output.
- `tests/equiv/config.rs::entry_and_map_types::rust_map_tables_dispatch_like_c_tables`
  passes a `static` Rust table, then four edits through `ConfigMapMut`, to C's
  `git_config_lookup_map_value`. Alongside it runs an independently built C table with its
  own strings. Over every prefix length and eight inputs, the codes, outputs and errors
  match.
