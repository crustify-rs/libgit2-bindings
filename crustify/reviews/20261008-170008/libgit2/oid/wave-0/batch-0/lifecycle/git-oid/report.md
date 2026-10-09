# `git_oid`: the header's typed null-id initializers have no constructor

- **item**: `git_oid` (type), `crustify/rust/libgit2/libgit2/oid/oid.rs`
- **affected**: construction paths of `Oid`
- **affected commit**: `36892fcd0` (libgit2/oid wave 0 batch 0; reviewed at `fc216e522`)
- **class**: lifecycle discovery — a published construction path without a Rust form;
  no UB or equivalence defect

## Finding

`include/git2/oid.h` publishes two value initializers for `git_oid` besides the
parsing routines:

```c
#define GIT_OID_SHA1_ZERO   { GIT_OID_SHA1, { 0 } }
#define GIT_OID_SHA256_ZERO { GIT_OID_SHA256, { 0 } }
```

together with their hex forms `GIT_OID_SHA1_HEXZERO` / `GIT_OID_SHA256_HEXZERO`. They
are the typed null ids that `git_oid_is_zero` recognises and that libgit2's own code
and tests start from. bindgen cannot express a brace initializer, so the `-sys` crate
has no binding for them, and none of the scheduled items covered them: the wrapper's
only zero value was `Oid::zeroed()` (`CZeroable`), which is libgit2's internal,
typeless `GIT_OID_NONE` (`src/libgit2/oid.h`), a value every formatter rejects as an
"unknown type". A consumer wanting the SHA1 null id had to assemble it by hand
(`zeroed()` + `set_kind`) or parse 40 zeros through libgit2 (`from_string`, fallible,
needing the runtime).

## Fix

`Oid::zero(kind: OidType) -> Oid`, an infallible, libgit2-free constructor producing
exactly the initializer's bytes (`kind` in the type byte, 32 zero id bytes), anchored
`Handwritten` and documented as the `GIT_OID_*_ZERO` / `GIT_OID_*_HEXZERO` value, as
the earlier `impl Default for OidType` documents `GIT_OID_DEFAULT` without a binding.

Evidence that it is the C value, byte for byte and as libgit2 sees it:

- `unit_tests::zero_is_the_typed_null_id` (Miri): the bytes, against `zeroed()` plus
  `set_kind`;
- `unit_tests::zero_formats_as_hexzero_and_is_zero`: `git_oid_is_zero`,
  `git_oid_tostr_s` giving the `HEXZERO` string, `git_oid_from_string` of that string
  giving the same value, and the typeless zero ordering below it;
- `tests/equiv/oid.rs::zero_ids_match_c`: a C `git_oid { type, { 0 } }` and
  `Oid::zero` compared byte for byte, through `git_oid_is_zero`, `git_oid_tostr_s`,
  `git_oid_from_string` and every comparison routine against every sample oid and hex
  string;
- `tests/ub_safe/oid.rs::sample_oids` now includes both null ids, so every formatter
  and comparison UB test runs over them in exact-size heap buffers.
