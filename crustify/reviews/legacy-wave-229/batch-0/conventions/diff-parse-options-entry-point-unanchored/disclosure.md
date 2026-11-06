To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: the git_diff_from_buffer_ext anchor sits on the convenience wrapper, not on from_buffer_with_options

Hi Marius,

This is a traceability and documentation problem, not a soundness bug. It can be handled in
public.

In `src/libgit2/src/libgit2/diff_parse.rs` (commit 0fcf482ab), the new
`Diff::from_buffer_with_options(content, opts: DiffParseOptions)` is the function that calls
`git_diff_from_buffer_ext` and exposes its whole surface. The anchor
``Wraps `fn::git_diff_from_buffer_ext` `` and its `#[doc(alias)]` stayed on
`from_buffer_with_oid_type`, which now only builds options and delegates:

```rust
/// Wraps `fn::git_diff_from_buffer_ext`
#[doc(alias = "git_diff_from_buffer_ext")]
pub fn from_buffer_with_oid_type(content: &[u8], kind: OidType) -> ... {
    Self::from_buffer_with_options(content, DiffParseOptions::with_oid_type(kind))
}
```

So a rustdoc search for `git_diff_from_buffer_ext` leads to a function that cannot take a
`DiffParseOptions`, and the anchor marks the wrong item as the C function's stand-in. The
campaign's conventions allow duplicate anchors only for ownership variants, and this is not
one.

The fix, which moves the anchor and alias to `from_buffer_with_options`, is on the review
branch (crustify/waves/wave-229). The report is under
`crustify/reviews/legacy-wave-229/batch-0/conventions/`. I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
