# `BackendEntryArc`'s "no mutation" compile-fail doctest cannot fail

- **item**: `git_config_backend_entry` (type), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `a6e60a4d5` (libgit2/config wave 2 batch 0)
- **class**: misc (regression evidence). The wrapper is sound; the test guarding it was not.

## Finding

The batch's evidence that an acquired backend entry cannot be mutated is this doctest
on `BackendEntry`:

```rust
/// ```compile_fail
/// use libgit2::libgit2::config::sys::config::BackendEntryArc;
/// fn mutate(mut entry: BackendEntryArc) { entry.as_mut(); }
/// ```
```

ffibox's `CArc` has no `as_mut` method under any policy. Its exclusive paths are
`get_mut` and `try_make_mut` / `make_mut` (bounded on `CRefClone`) and `try_into_box` /
`CBox::try_from` (also `CRefClone`). `as_mut` exists only on `CBox`, `CVal` and the
handles. So the snippet fails with E0599 whatever `BackendEntryFree` implements. It
would keep passing after exactly the regression it is meant to catch: a `CRefClone`
impl on the policy, which would hand out a `BackendEntryMut` over an entry that other
acquired references and the C list still share.

### Evidence

On `a6e60a4d5`, a temporary `CRefClone<BackendEntry>` impl was appended to
`BackendEntryFree` (`c_up_ref` and `c_is_sole_owner` both returning `true`), together
with a doctest that calls `entry.get_mut()`. Then
`cargo test -p libgit2 --doc -- sys::config::BackendEntry sys::config::probe` gives:

```
test libgit2/libgit2/config/sys/config.rs - libgit2::config::sys::config::probe_get_mut_compiles (line 646) ... ok
test libgit2/libgit2/config/sys/config.rs - libgit2::config::sys::config::BackendEntry (line 503) - compile fail ... ok
test libgit2/libgit2/config/sys/config.rs - libgit2::config::sys::config::BackendEntry (line 517) - compile fail ... FAILED
```

`get_mut` now compiles (the probe passes), yet the `mutate` doctest (line 503) still
passes. Only the `clone` doctest (line 517) notices the regression. The file was
restored after the probe.

## Fix

The `mutate` doctest now calls `entry.get_mut()`, and a new `unshare` doctest calls
`entry.try_into_box()`. These are the two ways a `CArc` gives up an exclusive handle.
All seven compile-fail doctests on the backend-entry surface now pin their error code
(`E0599`, `E0505`, `E0277`, `E0499`, `E0308`), so an unrelated error such as a renamed
import cannot satisfy them.

With the same temporary `CRefClone` impl on the fixed tree, `get_mut` (line 314),
`try_into_box` (line 319) and `clone` (line 333) all fail as compile-fail tests, so the
regression is caught. Without it, all seven pass.

## Noted, not changed

`EntryArc`'s doc in `config/config.rs` has the same vacuous `entry.as_mut()` doctest
(line 1299 on the wave tip `fb61c8337`). It belongs to type `git_config_entry`, which
wave 1 wrapped and reviewed, and is outside this workset, so it is left unchanged here.
The same fix applies: `get_mut()` / `try_into_box()`.
