# Backend-entry wrapper placed after `mod unit_tests`

- **item**: `git_config_backend_entry` (type), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `a6e60a4d5` (libgit2/config wave 2 batch 0)
- **class**: conventions (module layout)

## Finding

The batch appended the whole backend-entry wrapper (`define_ctype!`, `BackendEntryFree`,
`BackendEntryArc`, conversions and accessors) after the file's
`#[cfg(test)] mod unit_tests { .. }`. The test module ended up in the middle of the
file, between the memory-options wrapper and the backend-entry wrapper. Every other file
in `crustify/rust/libgit2/libgit2/` that has an inline `mod unit_tests` puts it last (a
scan of all of them for items after the module finds only this one). Readers looking for
the wrapper after the options had to skip the test module.

## Fix

The backend-entry wrapper now sits between the memory-options wrapper and
`mod unit_tests`, which is again the last item. The tests themselves stay in
`unit_tests::backend_entries`. rustfmt and clippy are clean on the file.
