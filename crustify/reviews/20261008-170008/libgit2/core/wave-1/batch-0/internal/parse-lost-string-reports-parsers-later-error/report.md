# `Signature::parse` reported the parser's later error, not the allocation failure, for a lost string

- **item**: `git_signature` → `Signature::parse` and its private helper `constructed`
  (`crustify/rust/libgit2/libgit2/core/types.rs`), from `git_signature_from_buffer`
- **affected commit**: `4dfa6b86a` (review wave base; the wrapper landed in `f62505e43`,
  core wave 1 batch 0)
- **kind**: internal — a private helper's assumption about libgit2's thread error state;
  no UB, nothing different reaches C

## Finding

`git_signature__parse` copies the name and the email with `extract_trimmed` without
checking the result (`src/libgit2/signature.c:350-351`), so an allocation failure sets
the out-of-memory error, leaves the field NULL, and the parse goes on and reports
success. `Signature::parse` refuses such a signature, and documents the refusal as
carrying "the out-of-memory error". The helper read it as the thread's *last* error:

```rust
if complete { Ok(signature) } else { Err(last()) }
```

But the parser keeps calling routines that set errors of their own after the lost
string and still succeeds. `git__strntol32` (`src/util/util.c:92`, via `git__strntol64`
at `:63` / `:71`) reports a timezone it cannot convert — `+abc` ("not a number"),
`+99999999999` ("too large") — with a `GIT_ERROR_INVALID` error, which
`git_signature__parse` then ignores: "malformed timezone, just assume it's zero"
(`signature.c:376`). For such a line the refusal carried that stale conversion error,
class `Invalid`, instead of the allocation failure the caller would need to react to.

## Evidence

`cargo run` in this directory (`LD_LIBRARY_PATH=/target/crustify/builds/plain
CRUSTIFY_BUILD_VARIANT=plain`) installs a libc allocator that refuses the one request a
counter names, lets `Signature::parse` initialise libgit2, then grants the struct and
refuses the name while parsing `A U Thor <a@b> 12 +abc`. On the affected commit:

```
with memory: Signature { name: Some("A U Thor"), email: Some("a@b"), when: Time { seconds: 12, offset_minutes: 0, sign: '+' } }
refused with class Some(Invalid), message "failed to convert string to long: not a number"

thread 'main' (2070700) panicked at src/main.rs:91:5:
assertion `left == right` failed: the refusal must carry the allocation failure that lost the name
  left: (Some(Invalid), "failed to convert string to long: not a number")
 right: (Some(NoMemory), "Out of memory")
exit=101
```

After the fix it prints `refused with class Some(NoMemory), message "Out of memory"`
and exits 0.

## Fix

`constructed` now calls `set_oom()` (libgit2's static out-of-memory error, which needs
no storage) before `last()` when a constructor's signature lacks its name or its email.
The only producer of such a signature is an allocation failure inside
`git_signature__parse`, so the error is the one that was set when the string was lost,
whatever the parser set afterwards. Its doc comment records why.

## Regression test

`unit_tests::signatures::parse_refuses_a_signature_libgit2_lost_a_string_of` (inline, in
the failing-allocator child) now also parses `A U Thor <a@b> 12 +abc` and
`A U Thor <a@b> 12 +99999999999` with the name refused, and asserts the
`NoMemory` / "Out of memory" error. It fails on the affected commit with the same
`left: Some(Invalid)` assertion.
