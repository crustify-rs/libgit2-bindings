To: Marius Momeu <marius.momeu@berkeley.edu>
Subject: libgit2-bindings: OdbBackendBox's adoption contract asks for a `free` member its policy does not need

Hi Marius,

This is a documentation problem in an `unsafe` adoption contract, not a soundness bug. It
can be handled in public.

In `src/libgit2/src/libgit2/odb.rs` (commit 9fe83bce6), the documentation of
`OdbBackendBox` says that `from_c`'s caller vouches that the backend is "complete, with its
`free` member set, and held by nothing else". The owner's policy does not need that:
`OdbBackendFree::c_drop` reads the member and leaves a backend without one alone (a leak,
which is safe):

```rust
if let Some(free) = unsafe { free_ptr.read() } {
    unsafe { free(backend) }
}
```

The batch's unit test `a_backend_without_free_is_left_alone` adopts exactly such a backend
through `from_c`, against the stated obligation. Meanwhile the obligation the owner does
rely on, that no odb holds the backend (an odb frees it again in `odb_free`), was only
implied.

The corrected wording, which asks for a complete backend held by nothing else, an odb
included, is on the review branch (crustify/waves/wave-181). The report is under
`crustify/reviews/wave-181/batch-0/contracts/`. I can send the exact diff on request.

Best regards,
Crustify

Found by [Crustify](https://github.com/crustify-rs/crustify), an experimental UB/soundness auditing agent developed at UC Berkeley and running on Claude Opus 5.5, then manually reviewed and independently reproduced.
