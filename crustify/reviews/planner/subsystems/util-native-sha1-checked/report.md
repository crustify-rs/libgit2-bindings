# util: sha1-checked, not sha1_smol, is the native SHA-1 equivalent

## What the earlier plan decided

```json
"name": "util",
"objective": {"objective": "wrap", "rust_native_equivalent": ["std", "sha1_smol", "sha2", "flate2", "regex"]}
```

## Evidence against it

The campaign's build hashes SHA-1 with collision detection. `crustify/artifacts/build.json`
configures `-DUSE_SHA1=builtin`, which compiles `src/util/hash/collisiondetect.c` and
`src/util/hash/sha1dc/` (both in `util`'s `implementation_files`). `git_hash_sha1_final`
there fails on a detected SHAttered-style collision. `sha1_smol` computes plain SHA-1 and
detects nothing, so replacing `util` with it would accept colliding objects that libgit2
rejects.

`sha1-checked` is the RustCrypto port of the same sha1dc algorithm. The registry copy in the
local cargo cache shows its metadata:

```sh
tar -xzOf ~/.cargo/registry/cache/index.crates.io-*/sha1-checked-0.10.0.crate \
  sha1-checked-0.10.0/Cargo.toml | sed -n '/^\[package\]/,/^\[/p' \
  | grep -E '^(name|version|description|license|rust-version) '
```

```text
rust-version = "1.72"
name = "sha1-checked"
version = "0.10.0"
description = "SHA-1 hash function with collision detection"
license = "MIT OR Apache-2.0"
```

Its README describes it as a "Pure Rust implementation of the SHA-1 cryptographic hash
algorithm with collision detection", using "the detection algorithm pioneered by git". Being
pure Rust, it builds on every platform the campaign targets, and it has the same licences as
the other entries. gitoxide's `gix-hash` 0.23.0 depends on it for its `sha1` feature
(`[dependencies.sha1-checked]` in its `Cargo.toml`, in the same cache), which keeps it
consistent with `oid`'s `gix-hash`.

## The edit

`util`'s `rust_native_equivalent` is `["std", "sha1-checked", "sha2", "flate2", "regex"]`;
the order of fit is unchanged. The objective stays `wrap`.

## What moves downstream

Nothing scheduled: `rust_native_equivalent` records a judgement, not a decision.
