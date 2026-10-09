# `Oid` is `Eq + Ord` but not `Hash`

- **item**: `git_oid` (type), `crustify/rust/libgit2/libgit2/oid/oid.rs`
- **affected**: `Oid`, `OidRef`
- **affected commit**: `36892fcd0` (libgit2/oid wave 0 batch 0; reviewed at `fc216e522`)
- **class**: misc (ergonomics); no UB or equivalence defect

## Finding

An object id is a key: libgit2 itself hashes `git_oid`s (`git_hashmap_oid_hashcode`,
the oid maps in `cache.c`, `odb.c`, `revwalk.c`), and a Rust consumer of this crate
will index objects by `Oid` as soon as it holds more than a handful. `Oid` and
`OidRef` implement `PartialEq` / `Eq` (`git_oid_equal`, a `memcmp` over all 33
bytes) and `Ord` (`git_oid_cmp`) but not `Hash`, so `HashSet<Oid>` / `HashMap<Oid, _>`
did not compile; a consumer had to wrap the value or key on `id()` and lose the type
byte that `Eq` distinguishes.

## Fix

`impl Hash for OidRef<'_>` over the bytes `Eq` compares — the type byte, then the 32
id bytes, read through the handle's pointer — and `impl Hash for Oid` through the
shared handle, so a value and its handle hash alike. The invariant `a == b ⇒
hash(a) == hash(b)` holds because `git_oid_equal` is `memcmp` over exactly those bytes.

Regression tests:

- `unit_tests::hash_follows_the_compared_bytes` (Miri): equal bytes hash equal on
  values and handles, a differing type byte or last id byte hashes differently;
- `unit_tests::oids_key_hashed_collections`: a `HashSet<Oid>` and a
  `HashMap<OidRef, _>` deduplicate by `git_oid_equal`;
- `tests/ub_safe/oid.rs::hashes_read_within_the_oid` (Miri, ASan): hashing boxed
  33-byte oids reads nothing outside them, on values, copies and handles.
