# `ReallocFn` / `Alloc::realloc` promise a liveness that a zero-byte resize breaks

- **item**: `git_allocator` (type), `crustify/rust/libgit2/libgit2/util/sys/alloc.rs`
- **affected**: the `# Safety` contracts of `ReallocFn` and `Alloc::realloc`, and the
  `Allocator::new` doc example that calls libc "an allocator which already meets
  `Alloc`'s contract"
- **affected commit**: `c647f7f01` (libgit2/util wave 0 batch 0)
- **class**: unsafe boundary — a stated safety obligation that is ambiguous for one
  input, and whose two readings lead a conforming caller or implementor to a double
  free; not a soundness defect of the safe API (no safe path reaches the members)
- **reproducer**: `src/main.rs` (ASan `double-free`; run as described in its header)

## Finding

`ReallocFn` is the type of `git_allocator.grealloc`, which `AllocatorRef::realloc`
hands out and `Allocator::new::<A>()` builds over `Alloc::realloc`. Both contracts
stated, before the fix:

> On success the old allocation is released and a new one holding its first
> `min(old, size)` bytes is returned; on failure NULL is returned and `ptr` stays
> live.

> `realloc(Some(ptr), len, ..)`, with `ptr` a live allocation of the implementation,
> either returns `None` and leaves `ptr` live and unchanged, or releases `ptr` and
> returns an allocation […]

C leaves `realloc(p, 0)` implementation-defined, and glibc (the libc every build here
runs on) releases `p` and returns NULL. The crate's own guidance tells implementors to
forward to libc — `Allocator::new`'s doc example wraps `malloc` / `realloc` / `free`
and calls the result "libc's allocator, which already meets `Alloc`'s contract" — and
libgit2's own default allocator (`src/util/allocators/stdalloc.c`) does the same. So
for `size == 0` the contract as written is false of the recommended implementation,
and a caller who follows it to the letter frees `ptr` a second time. The reproducer
does exactly that: it builds the doc example's allocator, takes its `realloc` and
`free` members, resizes a live 16-byte block to 0 and, reading NULL as "failure, `ptr`
stays live", releases `ptr`:

```
==787037==ERROR: AddressSanitizer: attempting double-free on 0x78741ede0010 in thread T0:
    #0 in free
    #1 in <Libc as libgit2::libgit2::util::sys::alloc::Alloc>::free src/main.rs:45:18
    #2 in libgit2::libgit2::util::sys::alloc::free_with::<Libc> crustify/rust/libgit2/libgit2/util/sys/alloc.rs:343:18
    #3 in alloc_realloc_contract_zero_size_resize::main src/main.rs:67:18

0x78741ede0010 is located 0 bytes inside of 16-byte region [0x78741ede0010,0x78741ede0020)
freed by thread T0 here:
    #0 in realloc
    #1 in <Libc as libgit2::libgit2::util::sys::alloc::Alloc>::realloc src/main.rs:42:31
    ...
previously allocated by thread T0 here:
    #0 in malloc
    #1 in <Libc as libgit2::libgit2::util::sys::alloc::Alloc>::malloc src/main.rs:33:31
    ...
SUMMARY: AddressSanitizer: double-free
```

The other reading — an implementor who takes the contract at its word and keeps `ptr`
live on a zero-byte resize, returning `None` — is also conforming, so the two sides of
the contract can disagree about who owns `ptr` afterwards. An implementor over
`std::alloc::GlobalAlloc` has a related gap: the contract never said whether `len`
can be 0 at all, and `GlobalAlloc::alloc` of a zero-size layout is undefined
behaviour, while libgit2 does make zero-byte requests (`git_odb_backend_data_alloc(b, 0)`
is `git__malloc(0)`; `git_attr_get_many` with no names is `git__calloc(0, n)`).

libgit2 never resizes a live allocation to zero bytes: every `git__realloc` /
`git__reallocarray` site (`str.c`, `vector.c`, `array.h`, `hashmap.h`, `oid.c`,
`tsort.c`, `refs.c`, `pack-objects.c`, `delta.c`, `sysdir.c`, `refdb_reftable.c`,
the OpenSSL and reftable allocator hooks, `xdl_realloc`) passes a size of at least
one byte when the pointer is non-NULL. So the ambiguity cannot be reached through
libgit2; it is reachable only by a direct caller of the member, and it is the
contract's text that is wrong.

## Fix

- `ReallocFn`'s `# Safety` now requires `size` to be non-zero when `ptr` is not
  NULL, says why (glibc's behaviour, which the old text would misread), and states
  that no libgit2 code path makes such a call. `Alloc::realloc`'s contract carries
  the matching clause, so an implementation need not decide the case.
- `MallocFn` and `Alloc::malloc` now say what a zero-byte request is: as C's
  `malloc(0)`, NULL or an allocation of its own that `free` releases, with libgit2
  reading NULL as out of memory. The `Allocator::new` example qualifies its claim.
- While there, the `Alloc` contract gained the clause that an implementation must
  not call into libgit2 (libgit2 allocates its per-thread error state through the
  allocator, so a call from inside it recurses without bound); this is a C-level
  hazard any `git_allocator` shares, recorded for implementors, not a Rust defect.
- Regression evidence: `tests/ub_unsafe/sys_alloc.rs::zero_byte_requests_are_allocations`
  exercises zero-byte requests through both allocating members within the fixed
  contract (ASan/LSan flag a lost or double-released block; Miri runs it over libc's
  model), and `unit_tests::zero_byte_requests_are_allocations` traces the same
  through `Alloc` under Miri. The reproducer's scenario itself is now outside the
  contract, so it is not a regression test.
