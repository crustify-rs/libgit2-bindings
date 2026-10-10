# Misc: the `git_blob_filter_flag_t` equivalence tests race libgit2's first filter init

**Item:** `git_blob_filter_flag_t` (wave 0 batch 0): `tests/equiv/blob.rs::filter_flags::{git_blob_filter_reads_rust_sets_as_c_flags, rust_sets_change_what_git_blob_filter_does}`.

**Finding.** Both drive `git_blob_filter` raw, which loads a filter list
(`git_filter_list__load`). libgit2 marks a registered filter initialised the first time a
list selects it, writing `fdef->initialized` under the registry's *read* lock
(`filter.c:344`), and every later load reads the flag (`filter.c:569`) under the same
read lock: two threads' loads around the first one are a data race (libgit2's defect, see
`filter.rs`'s module docs). The batch guarded against it with a `OnceLock` that has one
side run every call alone first — which orders the module's own loads, but not another
module's raw first load in the same test binary. The TSan run of the whole workspace on
the rebased tree reported exactly that: `equiv::checkout::Fixture::write_tree`'s
`git_index_add_bypath` (the writer, `filter.c:344`) against
`blob::filter_flags::Side::open`'s `git_blob_filter` (the reader, `filter.c:569`); two
earlier TSan runs of the same tree had been clean, so the race is a scheduling flake.

**Fix.** The two tests run alone in a child process (`isolated`, as the filter and
mailmap suites do), where the priming side's loads are the first in the process and every
other load follows them; the priming `OnceLock` stays, ordering the two sides within the
child. The remaining raw list loaders of other modules' fixtures (checkout, index,
`git_repository_hashfile`) can still race one another; they are those batches' tests.
