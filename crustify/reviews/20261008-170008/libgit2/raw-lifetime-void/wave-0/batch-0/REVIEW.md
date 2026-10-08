# Review: libgit2 / raw-lifetime-void / wave 0 / batch 0 (`void`)

Reviewed commit `9640ba589` (`OdbDataFree` / `OdbDataBox<T>`, `OdbDataArrayFree` /
`OdbDataVec<T>` over `git_odb_backend_data_alloc` / `git_odb_backend_data_free`).

| step | outcome |
|---|---|
| lifecycle discovery | complete: the only public `void *` primitives in libgit2.so are the two routines wrapped; `git_odb_backend_malloc` is deprecated (excluded, and removed by `DEPRECATE_HARD`); `git__malloc` / `git__free` / `git__calloc` are hidden. No gap. |
| wavefront findings | verified against `src/libgit2/odb.c`, `src/util/alloc.{c,h}`, `src/util/allocators/stdalloc.c`, `src/util/errors.c`, `src/util/runtime.c`; all correct, nothing resubmitted. |
| safe boundary | the workset adds no public `unsafe fn`; `runtime_pinned` is a private documented seam. `crustify scan-unsafe` reports no raw-pointer / void-pointer sites for the two routines and no undocumented unsafe. |
| UB | no candidate survived: plain, ASan/UBSan, TSan and BSan runs of the full workspace are clean on the reviewed commit. |
| equivalence | no defect: the traced-allocator sequences match step by step, including OOM error state. The two documented divergences (empty buffers never reach the allocator; the runtime pin corrects the pre-init TLS write) stand. |
| internal | no defect. |
| conventions | none; rustdoc builds with `-D warnings`. |
| misc | 1 report: `misc/miri-suites-abort-before-rust-only-tests` (fixed). |
| coverage | C: both routines 100% line coverage. Rust: only `data_alloc`'s "libgit2 could not be initialised" branch is uncovered (unreachable: `git_libgit2_init` cannot fail in this build). Added: an uninitialised-buffer clone/promotion UB test, a compile-fail for aliasing a box's handles, a Rust-only unit test over the policies' derived traits and the empty-buffer forms. |
