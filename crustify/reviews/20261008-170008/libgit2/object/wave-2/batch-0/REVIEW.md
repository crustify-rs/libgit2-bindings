# Review: libgit2 / object / wave 2 / batch 0 (the wave's types)

Reviewed commit `d6a6d1f95` (review wave base), covering the one `type` item the wave's
batches landed:

| item | wrapper | home |
|---|---|---|
| `git_commit_create_options` | `CreateOptions<'o>` value, `CreateOptionsRef` / `CreateOptionsMut` (batch 0, `8d42785bd`) | `object/commit.rs` |

All nine fields carry a `Field:` anchor (`version`, `allow_empty_commit`, `author`,
`committer`, `message_encoding`, `extra_headers` / `extra_headers_len`, `sign` /
`payload`), the type its `Wraps:` and `Handwritten` anchors, and
`git_commit_create_options_init` its `Wraps:` on `CreateOptions::new`.

Object's last wave is wave 3 (`schedule.json`), so this is not the sub-campaign's last
wave: no BSan gate here. No reproducer was emitted (no UB or equivalence defect found),
so no BSan run either.

| step | outcome |
|---|---|
| lifecycle discovery | complete, no gap. `git_commit_create_options_init` (and the `GIT_COMMIT_CREATE_OPTIONS_INIT` template it copies) is the type's only lifecycle routine: `grep "git_commit_create_options \*" include/git2` finds only it and the four routines that read the struct through a `const` pointer (`git_commit_create_from_stage`, `_from_tree`, `git_commit_amend_from_stage`, `_from_tree`), and `query symbols --lifetime-for git_commit_create_options` agrees. libgit2 never allocates, keeps, copies out or frees the struct or anything it points at (`create_from_stage` / `_from_tree` `memcpy` it into a local, `init_ext_options` copies four fields into an extended options local), so it has no dropper, disposer or cloner, and `CreateOptions` is rightly a Rust-held value without `Drop` or `Clone`. |
| wavefront findings | no defect. The type record and its six pointer-field records were read against `src/libgit2/commit.c` and hold: borrowed, never-stored signatures (NULL is the default signature, or the amended commit's author for `amend_from_tree`'s author), the borrowed encoding (overridden by the amended commit's when amending without a message), the borrowed header run read only when its length is non-zero, the `sign` callback copied into the extended options and its opaque `payload` (`mutable: null`: C never writes it), `allow_empty_commit` read only by the two creating routines. |
| safe boundary | clean. The workset's public surface has no `unsafe fn` and no raw pointer: `as_ptr` / `as_mut_ptr` are `pub(crate)`, the struct field is private, the signer is a `&'o mut SignatureCallback`. `crustify scan-unsafe --sites` finds no `*_smell`, `field_proj_outside_impl` or `raw_ptr_derefs_outside_impl` site in the type's code (the two in `commit.rs` are the signing and creation trampolines, symbol items); its 20 unsafe blocks and its `CCell` impl each carry a `// SAFETY:` comment within the scan's six-line window. No `ub_unsafe` suite applies. |
| UB | no candidate survived. Checked by hand: the bitfield accessors (bindgen's `_raw` accessors project the storage byte with `addr_of!`, no reference; `create_options_bitfield_keeps_its_neighbours` passes under Miri); `new`'s `assume_init` after `git_commit_create_options_init` (the C template's storage byte is a whole integer, so defined; padding needs no init); covariance of `CreateOptions<'o>` / `CreateOptionsRef` in `'o` and invariance of `CreateOptionsMut` (shrinking `'o` only shortens what the getters lend; a moved value keeps the stored borrows' regions); the signer's `&'o mut SignatureCallback<'f>` (requires `'f: 'o`; no getter reaches the payload; two options cannot share one signer, `E0499` doctest); `!Send` / `!Sync` through the `*const ()` marker; getters returning `SignatureRef<'o>`, `&'o CStr` and `CSlice<'o, Header<'o>>` independent of the handle borrow (each referent is borrowed for `'o` by its setter and immutable while shared). No public path hands the options to C yet: the wave-3 wrappers must take them exclusively, as `create_ext` does (re-entering a signer lent to options read through `&CreateOptions` would alias it). Instruments on the type's suites before and after the changes: plain, ASan/UBSan (LSan on) and TSan (`-Zbuild-std`) clean; Miri on its Rust-only inline tests: 2 passed (2 ignored: they call libgit2). |
| equivalence | no defect; gap filled: the equivalence script only reached the options through `git_commit_create_from_tree` / `git_commit_amend_from_tree`. `tests/equiv/commit.rs::create_options::commits_created_and_amended_with_the_options_match_c` now also commits each side's index (read from one of its trees, plus an entry making a tree no odb holds yet) with `git_commit_create_from_stage` and `git_commit_amend_from_stage` (allowlisted in `libgit2-sys` for the test): `GIT_EUNCHANGED` (`-38`) where the tree routine gives `-1`, default identities, encoding and headers, the amended commit's author, message and encoding, a refusing and a `GIT_PASSTHROUGH` signer; 8 more steps, each compared on code, class, message or masked commit, `HEAD`, signer calls and whether the staged tree was written. |
| internal | nothing to fix; the inline suite covers the private paths (`create_raw`, the bitfield neighbours, setter round trips with NULL clearing, libgit2 reading the struct in place). |
| conventions | none to fix: getters on `CreateOptionsRef`, setters on `CreateOptionsMut`, `addr_of!` / `addr_of_mut!` projections, Rust naming with the prefix dropped (`allow_empty_commit`, `set_signer`, `has_signer`), anchors as above. `cargo clippy --workspace --all-targets -D warnings` is clean; the touched Rust files pass `rustfmt +stable --edition 2024 --check`. |
| misc | 2 reports, fixed (docs only): `misc/create-options-docs-say-a-refused-signer-writes-nothing` — `set_signer` claimed a signer's error leaves nothing written, but `git_commit_create_from_stage` / `git_commit_amend_from_stage` write the index's tree before signing (shown by the new equivalence steps), and `allow_empty_commit` described only the tree routines' check; `misc/signature-callback-docs-omit-create-options` — `SignatureCallback::resume_panic` and the type docs named only the `git_commit_create_ext_options` route, though `CreateOptionsMut::set_signer`'s docs send callers to `resume_panic`. |
| coverage | Rust (`cargo llvm-cov`, `coverage` variant, the type's suites): no uncovered line in the `CreateOptions` code (`commit.rs` 3297–3768). C (gcov of the `coverage` build, private `GCOV_PREFIX`): `git_commit_create_options_init`, `init_ext_options`'s copy, both empty-commit checks (refused and allowed, tree and index), `create_from_tree`'s default-signature success and failure, and now `git_commit_create_from_stage` / `git_commit_amend_from_stage` are covered. Left uncovered, all error arms of the wave-3 symbols rather than reads of the options: `check_for_empty_commit`'s `HEAD`-tree and diff failures, `init_ext_options`'s NULL-options return (only a NULL `opts` reaches it), `git_repository_commit_parents`, `git_repository_index` and `git_index_write_tree` failures in the stage routines (environment-dependent). `git_commit_create_options_init`'s wrong-version refusal is unreachable from `CreateOptions::new`. |

## Validation of the landed tree

On the tree landed (base `d6a6d1f95`): `cargo check --workspace`, `cargo clippy
--workspace --all-targets -D warnings`, and `cargo test --workspace` under the plain,
ASan/UBSan (LSan on) and TSan (`-Zbuild-std`) variants: `lib` 578 passed / 1 ignored,
`equiv` 362, `ub_safe` 611 (`compile_fail` included), `ub_unsafe` 91, doctests 258, every
variant clean. Miri on the type's Rust-only inline tests: 2 passed. `cargo doc` adds no
warning (the two it reports are outside `commit.rs`). No C changed: the only `-sys`
change is the allowlist entry for the two stage routines.

## Noted, not changed

- **The wave-3 wrappers of the four routines** (`create_from_stage` / `_from_tree`,
  `amend_from_stage` / `_from_tree`) should take `CreateOptionsMut` (or `&mut`), not
  `&CreateOptions`, and resume the signer's panic after C returns, as `create_ext` does;
  refuse signatures that lost a string (`commit::writable`); and replace
  `tests/ub_safe/fixture.rs::create_options` and the raw calls in the equivalence test.
