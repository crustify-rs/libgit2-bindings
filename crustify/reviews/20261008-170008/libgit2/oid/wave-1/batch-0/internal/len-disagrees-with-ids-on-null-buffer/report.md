# `len()` reported `count` while every other accessor read a NULL buffer as empty

- **item**: `git_oidarray` (type), `crustify/rust/libgit2/libgit2/oid/oidarray.rs`
- **affected**: `OidArrayRef::len` (and `is_empty`), against `ids`, `ids_mut`, `take_ids`,
  `Debug` and (after the `ub_safe` fix) `OidArrayDispose`
- **affected commit**: `b70e52aae` (libgit2/oid wave 1 batch 0; reviewed at `8cb33ad6b`)
- **class**: internal consistency; no UB (no safe path produces the state) and no
  change for any in-contract array

## Finding

The handles' invariant (`ids` NULL with `count == 0`, or a buffer of at least `count`
ids) is vouched for by the crate-internal `from_ptr`, not checked. For a struct that
breaks it with `ids == NULL` and `count > 0` — the state libgit2's own
`git_smart__shallow_roots` would leave on an out-of-memory `git__malloc`, were it not
for the `memcpy` into NULL that crashes first — the wrapper deliberately tolerated the
state rather than dereferencing NULL: `ids()` / `ids_mut()` returned an empty run
(a run may not start at NULL), `take_ids()` an empty vector, `Debug` printed `[]`. But
`len()` returned `count`, so a handle could report `len() == 3` with
`ids().len() == 0`, and the batch's own `unit_tests::handles_over_a_c_owned_array`
asserted exactly that pair while describing the array as one that "reads as empty".
A consumer indexing by `len()` would panic on `get(i).unwrap()`; one summing lengths
would be wrong.

## Fix

`len()` now reads `ids` as well and returns 0 when it is NULL, so every accessor —
and the dispose policy, which also skips a NULL buffer — agrees that an array without
a buffer is the empty array. In-contract arrays (`count == 0` whenever `ids` is NULL)
read exactly as before; the module docs state the rule.

Regression tests:

- `unit_tests::null_buffer_is_empty_and_disposes_without_c` (Miri): over a bare
  `{ NULL, 3 }`, `len()` is 0, the runs are empty, `Debug` is `[]`, and the policy
  releases nothing;
- `unit_tests::handles_over_a_c_owned_array` now asserts the consistent reading and
  that `truncate` leaves such a struct alone.
