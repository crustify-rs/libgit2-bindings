# Regex filter callbacks project the payload outside its impl

- **item**: `git_config_iterator` (type): the Rust-owned regex-filtered multivar
  iterator (`value_filter::Filtered`), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `0de2b59b4` (libgit2/config wave 3 batch 0)
- **class**: unsafe (scan-unsafe smell)

## Finding

`value_filter`'s `next` and `free` callbacks were module-level `unsafe extern "C" fn`s.
`next` reached the Rust state of the enclosing allocation with
`&mut *addr_of_mut!((*iterator.cast::<Filtered<'_>>()).state)` from outside any impl of
`Filtered`. `crustify scan-unsafe --sites all` counted this at `config/sys/config.rs:932`
as one `field_proj_outside_impl` and two `raw_ptr_derefs_outside_impl`. These were the
workset's only `*_outside_impl` sites. The payload also embedded the raw
`ffi::git_config_iterator` base, not the module's `Iterator<'c>` layout type. The
conventions reserve the by-value layout type for exactly this case: Rust-owned storage
that embeds the object.

## Fix

`next` and `free` are now associated functions of `impl Filtered<'_>`. They cast to
`Self`, so the projection lives with the type whose layout it relies on. The payload's
base is `Iterator<'c>`, built from the same C struct value (offset zero, `#[repr(C)]`
payload, `#[repr(transparent)]` base). The callbacks are installed as `Filtered::next` /
`Filtered::free`. The logic is unchanged. `filtered_multivar_steps_match_c` (new) compares
the filter step by step with C's own regex multivar iterator, including a refresh failure
and retries. `filtered_iterator_release_points` (new) releases the payload before, during
and after iteration. Both pass under plain, ASan/UBSan (LSan on) and TSan. Scan:
`field_proj_outside_impl` 5 → 4, `raw_ptr_derefs_outside_impl` 17 → 15. The payload stays
one of the crate's three `wrapper_newtypes_undeclared` sites, the same shape as
`core/types.rs`'s `Repr<W>` stream payload: an enclosing allocation, not a wrapper, so
no `CCell` is declared for it.
