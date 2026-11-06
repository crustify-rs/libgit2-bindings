# `Buf::from_raw`'s safety contract did not require the `size` bytes to be initialised

- **item**: `git_buf` (type), `crustify/rust/libgit2/libgit2/core/buffer.rs`,
  `pub unsafe fn Buf::from_raw(raw: ffi::git_buf) -> BufVal`
- **affected commit**: `cfcd11d5c` (libgit2/core wave 0 batch 0; reviewed at `ab5de9104`)
- **class**: unsafe API with an incomplete `# Safety` statement; no reproducer — the
  contract is strengthened, not the code

## Finding

`from_raw` is the one public `unsafe fn` the workset adds, and it legitimately stays
unsafe: it adopts ownership of an allocation C filled, which no type can prove. Its
contract said, for `reserved > 0`, that `ptr` is a live allocation of at least
`reserved` bytes from libgit2's current allocator, owned and given up, with
`size < reserved` and a NUL at `ptr[size]`. It did not say that the first `size` bytes
are initialised, nor that nothing else accesses the allocation while the owner lives.
Both are relied on: `BufRef::as_bytes` forms `&[u8]` over `ptr[0..size)`
(uninitialised bytes behind a `&[u8]` are UB on the Rust side) and the handles' `Sync`
proof assumes no concurrent foreign writer. A caller building a `git_buf` by hand —
`git__malloc(reserved)` and a NUL at `size` without filling the rest, which the
clause "as libgit2's routines leave a buffer they filled" only implied was excluded —
would satisfy the letter of the old contract and reach UB through safe calls.

The `reserved == 0` clause was also silent on what the wrapper does with `ptr` and
`size`: it never reads through them, which the caller needs to know to pass a
`reserved == 0` struct with a foreign `ptr` (libgit2's static empty string) safely.

## Fix

The `# Safety` section now requires the `size` bytes initialised and the allocation
untouched by anyone else while the result lives, and states that with `reserved == 0`
neither `ptr` nor `size` is read through. The existing `ub_unsafe/buffer.rs` suite
already builds every adopted buffer within the strengthened contract
(`c_buf` fills the bytes it reports), so no test changed.
