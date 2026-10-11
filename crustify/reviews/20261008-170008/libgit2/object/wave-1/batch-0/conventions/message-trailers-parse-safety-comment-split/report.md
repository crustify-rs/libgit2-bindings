# `MessageTrailerArray::parse`: `// SAFETY:` separated from its `unsafe` block by a rustfmt line break

- item: `git_message_trailer_array` (wave 1 batch 2,
  `crustify/rust/libgit2/libgit2/object/message.rs`)
- kind: conventions — the campaign's unsafe scan counted the block as undocumented
- affected commit: `635f83bbf`, reviewed at `38e6d1b3e`

## Finding

`crustify scan-unsafe` reports two undocumented unsafe sites in `object/message.rs`. One
is `MessageTrailerArray::parse`'s call to `git_message_trailers`: its `// SAFETY:` comment
is there (`clippy::undocumented_unsafe_blocks` accepts it), but rustfmt broke
`let rc = unsafe { … };` over two lines, so `let rc =` stands between the comment and the
`unsafe` keyword, which the scan's lexical rule does not look past (the same pattern
`core/wave-2/batch-0/conventions/` fixed in `equiv/trace.rs`). The other site in the file
(`MessageTrailerRef::from_ptr`, line 330) predates this wave.

## Fix

The array's pointer is bound first (`let place = array.as_mut().as_mut_ptr();`), so the
call fits one line directly under its comment, which now names `place`. No behaviour
change; the scan's count for the file drops from 2 to 1. The new test code of this review
was kept to the same shape (`tests/equiv/commit.rs` is back to its count before the
review).
