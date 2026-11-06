# `OdbBackendBox`'s adoption contract asks for a `free` member its policy does not need

- **objective**: `contracts` (an obligation asking for more than the implementation needs;
  no reproducer, as `verification/reviewing.md` allows for this kind)
- **where**: `src/libgit2/src/libgit2/odb.rs`, the documentation of `OdbBackendBox`
  (`:3398`–`:3406` at `9fe83bce6`), against `OdbBackendFree`'s `CDrop` impl (`:3377`–`:3394`)
  and the unit test `unit_tests::backend::a_backend_without_free_is_left_alone`
- **introduced by**: wave 180 batch 0 (`9fe83bce6`)

## What is wrong

Until the backend constructors are wrapped, `OdbBackendBox::from_c` is the only way to
build an owner. The alias documents what that `unsafe` call's caller vouches for:

```rust
/// … Until those are wrapped, an owner is adopted
/// with `from_c`, whose caller vouches that the pointer is such a backend, complete,
/// with its `free` member set, and held by nothing else. …
pub type OdbBackendBox = CBox<OdbBackend, OdbBackendFree>;
```

The policy the owner drops through does not need the `free` member. It reads it and leaves
a backend without one alone:

```rust
// SAFETY: … A backend without
// one cannot be released, and is left alone, where libgit2 would call through NULL.
unsafe impl CDrop<OdbBackend> for OdbBackendFree {
    unsafe fn c_drop(&self, ptr: NonNull<OdbBackend>) {
        …
        if let Some(free) = unsafe { free_ptr.read() } {
            unsafe { free(backend) }
        }
    }
}
```

The batch's unit test `a_backend_without_free_is_left_alone` adopts a backend without
`free` through `from_c` and checks that dropping the owner releases nothing. It violates the
alias's stated obligation, while its own SAFETY comment ("with no `free` member the owner
releases nothing") follows the implementation. The obligation and the policy disagree,
and the stricter one is not needed for soundness. Dropping a member-less backend leaks,
which is safe. Every member call through `OdbBackendMut` already handles an unset member.
The requirement that *is* needed, that no odb holds the backend (an odb would free it
again in `odb_free`), was only implied by "held by nothing else".

## Fix

The alias's documentation now asks for what the policy and the handles rely on: a
backend that is complete and held by nothing else, an odb included. It also states what
happens to one without `free`: the owner leaves it alone, and its storage stays its
builder's. The `CDrop` impl and the unit test already match this wording. No code changes.
