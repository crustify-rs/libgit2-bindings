# `Diff::from_buffer_with_options` stands in for `git_diff_from_buffer_ext` without its anchor

- **objective**: `conventions`
- **where**: `src/libgit2/src/libgit2/diff_parse.rs`, `Diff::from_buffer_with_options` and
  `Diff::from_buffer_with_oid_type`
- **introduced by**: wave 228 batch 0 (`0fcf482ab`, "wrap git_diff_parse_options as
  DiffParseOptions")

## What is wrong

Wave 228 batch 0 added `Diff::from_buffer_with_options(content, opts: DiffParseOptions)`. It
is now the only function that calls `ffi::git_diff_from_buffer_ext`, and it exposes the C
function's whole surface: the content and every field of the parse options.
`Diff::from_buffer_with_oid_type(content, kind)` became a convenience. It builds
`DiffParseOptions::with_oid_type(kind)` and delegates. The batch still left the anchor
on the convenience:

```rust
    /// Parse with an explicit object-id type.
    /// ...
    /// Wraps `fn::git_diff_from_buffer_ext`
    #[doc(alias = "git_diff_from_buffer_ext")]
    pub fn from_buffer_with_oid_type(content: &[u8], kind: OidType) -> ... {
        Self::from_buffer_with_options(content, DiffParseOptions::with_oid_type(kind))
    }

    /// Parse with the given options: the entire parse-options surface of
    /// `git_diff_from_buffer_ext`. ...
    pub fn from_buffer_with_options(content: &[u8], opts: DiffParseOptions) -> ... {
```

The conventions' Anchors section ("What is annotated") puts the annotation on "the
function or method the public API exports, which a consumer imports instead of the FFI
function". A consumer porting `git_diff_from_buffer_ext(&out, buf, len, &opts)` imports
`from_buffer_with_options`: only it takes the options value. A second annotated stand-in is
allowed only for the ownership variants one signature cannot express ("Several
stand-ins"). `from_buffer_with_oid_type` is not one of those: it is a subset of the same
contract.

Effect: rustdoc's alias search for `git_diff_from_buffer_ext` finds only
`from_buffer_with_oid_type`. That function cannot pass a `DiffParseOptions`, so the
type the batch wrapped is undiscoverable from the C name, and the anchor that records the
translation sits on the wrong item.

## Evidence

```sh
grep -n -B3 'pub fn from_buffer_with' src/libgit2/src/libgit2/diff_parse.rs
grep -n 'git_diff_from_buffer_ext' src/libgit2/src/libgit2/diff_parse.rs
```

At the wave tip (`9872835e0`), the first command shows the `Wraps`/`doc(alias)` pair above
`from_buffer_with_oid_type` only. The second shows the one FFI call inside
`from_buffer_with_options`. No reproducer: a convention deviation needs none.

## Fix

The anchor and alias move to `from_buffer_with_options`. `from_buffer_with_oid_type`
keeps its documentation, which now points at `from_buffer_with_options` for the full
surface. One item carries the anchor, so the count of wrapped symbols is unchanged.
