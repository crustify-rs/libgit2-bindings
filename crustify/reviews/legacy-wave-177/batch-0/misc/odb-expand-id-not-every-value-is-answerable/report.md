# `OdbExpandId`'s docs say libgit2 answers every value safely; a full-length query of another id type overflows the heap

- **type**: `git_odb_expand_id`, bound as `OdbExpandId` (`src/libgit2/src/libgit2/odb.rs`)
- **affected commit**: `603bf0085` (wave 176 batch 0), and its commit message ("Every value is
  valid: the routine reads no more than the 32 id bytes whatever the length says")
- **objective**: `misc` (documentation that the next wrapper relies on). No safe path reaches
  `git_odb_expand_ids` yet, so this needs no UB reproducer. The behaviour was checked under
  ASan with a raw call (below).

## What is wrong

The type's docs said:

> Every value is valid for the type: libgit2 reads no more than the 32 id bytes whatever
> `length` says, and a `type` it does not know matches nothing.

That holds for the struct's own bytes, and for a prefix query. It does not hold for the only
routine that consumes the struct. `git_odb_expand_ids` (`c/src/libgit2/odb.c:894`) takes an
entry whose `length` is at least the odb's hex size as a full id, and resolves its type with
`odb_otype_fast` → `odb_read_header_1` → the backends' `read_header`, without comparing the
id's type with the odb's. The loose backend formats the id into a path sized for the odb's
type (`object_file_name`, `c/src/libgit2/odb_loose.c:82`). This is the out-of-bounds
formatting that the module docs describe for the readers, which the readers' wrappers refuse.
A query carrying a SHA-256 id with `length` 64, handed to a SHA-1 odb, therefore writes past
the path buffer. A SHA-1 id in a SHA-256 odb, or a typeless id in either, leaves part of the
path unwritten.

An `OdbExpandId` built through the safe API and passed to the raw routine (the call batch 0's
equivalence tests make, and the one the routine's later safe wrapper will make), run from a
scratch package against the `asan-ubsan` variant:

```rust
let odb = Odb::open(c"/tmp/exc")?;                                   // SHA-1
let wide = Oid::from_raw(&[0xab; 32], OidType::Sha256)?;
let mut q = [OdbExpandId::new(wide.as_ref(), 64, None)];
unsafe { ffi::git_odb_expand_ids(odb.as_ref().as_ptr().cast_mut().cast(), q.as_mut_ptr().cast(), 1) };
```

```
ERROR: AddressSanitizer: heap-buffer-overflow on address 0x7209d3a12e20 ...
WRITE of size 1 at 0x7209d3a12e20 thread T0
    #0 git_oid_fmt_substr c/src/libgit2/oid.h:133:10
    #1 git_oid_pathfmt c/src/libgit2/oid.c:153:2
    #2 object_file_name c/src/libgit2/odb_loose.c:94:2
    #3 locate_object c/src/libgit2/odb_loose.c:466:14
    #4 loose_backend__read_header c/src/libgit2/odb_loose.c:608:6
    #5 odb_read_header_1 c/src/libgit2/odb.c:1007:11
    #6 odb_otype_fast c/src/libgit2/odb.c:1196:10
```

The docs' claim would lead the author of the `git_odb_expand_ids` wrapper, a later symbol
batch per the commit message, to pass caller-built queries through unchecked.

## Fix

The paragraph now says that every value is valid for the type itself (its accessors move
plain bytes, and nothing in the crate hands one to libgit2 yet), but that not every value is
a query `git_odb_expand_ids` answers safely. It names the full-length path, the odb's
unchecked id type, the overflow and the unwritten bytes, and states the obligation on the
routine's wrapper: refuse a full-length query whose id is not of the odb's type. Documentation
only. No code or ffimap counter changes.
