# `MessageTrailerArray::parse`: libgit2's unchecked trailer-buffer growth is undocumented

- item: `git_message_trailer_array` (wave 1 batch 2, `MessageTrailerArray::parse`)
- kind: misc — documentation of a confirmed C defect
- affected commit: `635f83bbf`, reviewed at `38e6d1b3e`

## Finding

`parse` documents that it fails "with libgit2's error, when the block cannot be
allocated". That covers `extract_trailer_block`'s allocation only. `git_message_trailers`
(`src/libgit2/trailer.c`, `S_VALUE_END`) then grows its trailer buffer with
`git_array_alloc`, which yields NULL when `git_array__alloc` fails (it frees the old
buffer and zeroes the sizes, `src/util/array.h:44-77`), and writes `t->key` / `t->value`
through it unchecked: an allocation failure there is a NULL write, not an error. The batch's equivalence test makes only the block's
allocation fail, so the docs read as if every allocation failure were reported.

No safe API of the crate installs a failing allocator (only the raw
`GIT_OPT_SET_ALLOCATOR` code exists), so reaching it from safe Rust takes real memory
exhaustion after the block was allocated; it is not a UB defect of the wrapper, and
avoiding it would mean re-implementing the parser in Rust, which a wrap campaign does
not do.

## Fix

The `parse` docs now say that libgit2 does not check that growth and crashes should it
fail.
