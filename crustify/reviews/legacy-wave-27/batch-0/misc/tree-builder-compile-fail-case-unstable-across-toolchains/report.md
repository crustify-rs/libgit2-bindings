# Misc: `compile_fail/tree_builder_not_send_or_clone` failed under the sanitizer toolchains

**Item:** `git_treebuilder` (wave 0 batch 4), `tests/ub_safe/compile_fail/tree_builder_not_send_or_clone.rs`

**Finding.** The trybuild case checked three things at once: a `TreeBuilderRef` is not
`Sync`, a `TreeBuilderBox` is not `Send`, and the box does not `clone`. Its `.stderr`
was recorded with the stable toolchain. The `E0277` diagnostics for the two thread
checks differ under the nightly toolchains `cargo-variant.sh asan-ubsan` and `tsan`
use: nightly dedups the second `*const ()` chain for `Sync` and prints a source snippet
for `PhantomData` in the `Send` chain. So `cargo-variant.sh asan-ubsan test --workspace`
and the TSan run both failed in `ub_safe::compile_fail` (`1 of 116 tests failed`,
`tree_builder_not_send_or_clone.rs ... mismatch`) on the reviewed wave tip, before any
change of this review, while the plain run passed. This is the pitfall the
`cargo-variant` notes record (prefer `compile_fail,E0277` doctests for `Send` / `Sync`).

**Fix.** The `Send` and `Sync` checks are already `compile_fail,E0277` doctests on
`TreeBuilder` (the owner sent to a thread, `TreeBuilderRef` required `Sync`), which
rustdoc checks by error code alone, so the trybuild case now keeps only the `clone`
check, whose `E0599` output is identical on every toolchain:
`tests/ub_safe/compile_fail/tree_builder_not_clone.rs`. Verified: the `compile_fail`
test passes on plain, ASan/UBSan and TSan.
