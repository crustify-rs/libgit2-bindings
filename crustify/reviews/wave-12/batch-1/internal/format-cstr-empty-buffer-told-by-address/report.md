# `OidRef::format_cstr` tells an empty buffer by its address, not its length

- **item**: `git_oid_tostr` → `OidRef::format_cstr` (`crustify/rust/libgit2/libgit2/oid/oid.rs`)
- **affected commit**: `8cb33ad6b` (wave base; the wrapper landed in `ed8e139b2`, oid wave 1 batch 1)
- **kind**: internal — a private decision in the wrapper that the C contract does not support; no UB, no divergence in what reaches C

## Finding

`git_oid_tostr(out, n, oid)` returns libgit2's static `""` when `n == 0` (or `out` is
NULL) and `out` itself otherwise (`src/libgit2/oid.c:217`). The wrapper used the
returned pointer to recognise the first case:

```rust
let rc = unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), self.as_ptr()) };
if rc.cast_const() != out.as_ptr().cast() {
    return Ok(c"");
}
...
Ok(CStr::from_bytes_until_nul(out).expect("git_oid_tostr NUL-terminates within its buffer"))
```

That works only because an empty slice's address *usually* differs from the static
string's. The address of a zero-length `&mut [u8]` carries no information: it may be a
dangling `NonNull::dangling()`, one past a Rust buffer, or any non-null aligned pointer a
caller obtained soundly — including libgit2's `""` itself. When the two coincide the
wrapper takes the "written" path, finds no NUL in zero bytes and panics with a message
blaming libgit2 (`git_oid_tostr NUL-terminates within its buffer`). The real condition
C acts on is the length, which the wrapper has in `out.len()`.

## Evidence

`cargo run` in this directory (`LD_LIBRARY_PATH=/target/crustify/builds/plain
CRUSTIFY_BUILD_VARIANT=plain`) obtains the static string's address from a C call with
`n == 0`, builds a sound zero-length slice at that address (no byte is read or written
through it) and calls the safe wrapper. On the affected commit:

```
static empty string at 0x7a655f3cafd6

thread 'main' (1152205) panicked at .../crustify/rust/libgit2/libgit2/oid/oid.rs:533:18:
git_oid_tostr NUL-terminates within its buffer: FromBytesUntilNulError(())
```

After the fix it prints `Ok("")` and exits 0.

## Fix

`format_cstr` now returns `Ok(c"")` when `out.is_empty()` before calling C (C would
touch neither the buffer nor the thread's error for that length, so nothing observable
changes), and `debug_assert`s that a non-empty call returns `out`. The doc comment now
says the empty case reports nothing whatever the type.

Regression: `unit_tests::format_cstr_tells_an_empty_buffer_by_its_length` (inline,
since it needs the unsafe zero-length slice): an empty slice at the static string, a
literal `&mut []`, and an empty tail of a real buffer all give `Ok("")` for both known
types, for the typeless zero and for an unnamed type byte, leave a primed thread error
in place and write nothing. It fails on the old logic with the panic above.
