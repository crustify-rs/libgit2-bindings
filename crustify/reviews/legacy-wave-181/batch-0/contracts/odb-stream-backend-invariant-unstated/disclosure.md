To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: OdbStreamRef::backend relies on a `backend` invariant no contract states

Hi Marius,

This is a contract-documentation gap, not a bug reachable from the safe API. No safe path
builds a stream with a foreign `backend`, so it can be handled in public.

Since commit 9fe83bce6, `OdbStreamRef::backend` and `OdbWritepackRef::backend`
(`src/libgit2/src/libgit2/odb.rs`) lend the C struct's `backend` field as an
`OdbBackendRef`, whose safe getters read through it. Before, they returned a raw pointer.
Their SAFETY comment cites "the owner's adoption", but neither `from_c` nor the handles'
`from_ptr` asks the caller to vouch for that field. `from_ptr`'s obligation (ffibox's
`ref_from_ptr`) only asks for a live, initialised `git_odb_stream`. Honouring exactly that,
safe code reaches UB under Miri:

```rust
let mut stream = ffi::git_odb_stream { backend: freed_backend, ..Default::default() };
let handle = unsafe { OdbStreamRef::from_ptr(&raw mut stream) }.unwrap();
handle.backend().map(|b| b.version()); // UB: memory access failed: alloc307 has been freed
```

The batch's own unit test adopts a stream whose `backend` is the provenance-less address
`0x1000`, and calls `backend()`.

The fix on the review branch (crustify/waves/wave-181) states the invariant on both types,
completes the SAFETY comments, and gives the unit test a live backend. I can send the full
reproducer on request. The report is under
`crustify/reviews/legacy-wave-181/batch-0/contracts/`.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
