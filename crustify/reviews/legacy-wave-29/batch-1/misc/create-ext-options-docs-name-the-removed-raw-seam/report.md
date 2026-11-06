# `CreateExtOptions` docs still justify `!Send` / `!Sync` by the raw signing seam batch 5 removed

- **item**: `git_commit_signature_cb` (the change that left them stale), in
  `crustify/rust/libgit2/libgit2/object/commit.rs` and
  `crustify/rust/libgit2/tests/ub_safe/commit.rs`
- **affected commit**: `846e7d391` (libgit2/object wave 1 batch 5)
- **class**: documentation defect

## Finding

Batch 5 replaced `CreateExtOptionsMut::set_sign_callback`, batch 1's `pub unsafe`
raw seam taking a C callback and a `*mut c_void` payload, with the safe `set_signer`,
which borrows a `SignatureCallback` for the options' `'o`. Two texts kept describing
the removed seam:

- the type docs of `CreateExtOptions`: "Neither `Send` nor `Sync`: the signing
  callback's raw seam stores a payload whose thread affinity the caller alone knows";
- the docs of `tests/ub_safe/commit.rs::create_ext_options`: "… aliased handles and
  the raw seam without `unsafe` do not compile", whose compile-fail case batch 5
  replaced with `create_ext_options_signer_outlives_options`.

## Fix

The type docs now give the actual reason: the payload is a `SignatureCallback`
borrowed exclusively, whose closure need not be `Send`, called on the thread writing
the commit. The test module docs name the signer borrow check instead of the seam. No
behaviour changes.
