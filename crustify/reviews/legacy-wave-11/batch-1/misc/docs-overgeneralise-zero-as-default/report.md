# `oid.rs` documents `0` as "the default type" in every options struct

- **item**: `git_oid_t` (`OidType`)
- **affected**: `crustify/rust/libgit2/libgit2/oid/oid.rs` — the module doc, the
  `OidType` doc, the `impl Default for OidType` doc and the `UnnamedOidType` doc
- **affected commit**: `736863259` (wave 0 batch 1)
- **class**: misc — documentation states a contract libgit2 does not keep; no code
  defect

## Finding

The wrapper's docs, following the wavefront note corrected in
`wavefront/options-zero-is-not-always-default`, told API consumers that

> a zeroed `oid_type` field of an options struct means "the default type"
> (`OidType::default`)

and that `0` is a value "an options struct reads as the default, or a value libgit2
rejects". Both statements are the reasoning a later batch wrapping an options struct
will read before deciding what an unset field maps to, and both are wrong for
`git_diff_options` (0 selects the repository's type; `diff_generate.c:502`) and
silent about the routines that *return* `0` (`git_repository_oid_type` without a
repository, `git_remote_oid_type` on failure), which are the reason `from_raw` must
be fallible for results and not only for inputs.

`OidType` itself is correct: it names exactly `GIT_OID_SHA1` and `GIT_OID_SHA256`,
`from_raw` / `TryFrom` refuse every other value, `Default` is `GIT_OID_DEFAULT`,
`size` / `hex_size` match `git_oid_size` / `git_oid_hexsize`, and no `unsafe` or raw
pointer is involved (`crustify scan-unsafe --name git_oid_t`: no sites).

## Fix

The four doc comments now say that `0` selects the default "where no repository
supplies its own" (naming `git_diff_options` as the exception) and that libgit2's
routines returning a type answer `0` when they have none.

Regression evidence, which also fills the coverage gap on the `git_oid_t` C paths no
oid-subsystem call reaches: `tests/equiv/oid.rs::repository_oid_types_follow_the_rust_type`
creates a bare repository with each option value (`0`, `GIT_OID_SHA1`,
`GIT_OID_SHA256`), predicts `git_repository_oid_type`'s answer from
`OidType::from_raw(raw).unwrap_or_default()`, checks that only a non-default type is
recorded in the config (`git_oid_type_name`), reopens the repository
(`git_oid_type_fromstr`), rewrites the format to `sha512` and checks the open fails
with `GIT_EINVALID` and a `0` type, rewrites it to `sha1` (which libgit2 never writes
itself) and checks it reads back as the default, checks `git_repository_oid_type(NULL)` is a value
`from_raw` refuses, and records the C-only `git_repository_init_ext` quirk for an
unnamed value. It needed `git_repository_init_ext`, `git_repository_init_options_init`
and `git_repository_oid_type` (with `git_repository_init_options`,
`git_repository_init_flag_t` and `GIT_REPOSITORY_INIT_OPTIONS_VERSION`) added to the
`libgit2-sys` allowlists.
