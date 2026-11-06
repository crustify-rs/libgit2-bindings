# `OdbDataStr` adoption guidance omits that a `git_buf` owns its `ptr` only while `reserved > 0`

- **item**: `string` (raw-lifetime), `crustify/rust/libgit2/libgit2/odb/sys/odb_backend.rs`
- **affected**: `OdbDataStrFree` / `OdbDataStr` (`CStrBox<OdbDataStrFree>`), its docs and
  the module docs
- **affected commit**: `68cd137c7` (wave 0 batch 0)
- **class**: unsafe boundary — an ambiguous adoption contract for a string handle
  libgit2 hands out; not a soundness defect of the safe API
- **reproducer**: `src/main.rs` (ASan `bad-free`; run as described in its header)

## Finding

The only way a libgit2-made string enters an `OdbDataStr` is `CStrBox::from_raw`,
whose generic contract is "a valid, NUL-terminated, uniquely-owned string that
`D::c_drop` releases". The workset's docs tell the consumer which strings those are:
the module header says the strings "`git_strarray_dispose`, `git_signature_free` and
`git_buf_dispose` release with `git__free`", and `OdbDataStrFree`'s doc repeats it.
For `git_buf` that is only half true. `buf.c`:

```c
void git_buf_dispose(git_buf *buf)
{
	...
	if (buf->ptr != git_str__initstr)
		git__free(buf->ptr);
	...
}
```

A `git_buf` libgit2 fills owns its `ptr` only when `reserved > 0`; an empty result is
produced without allocating and points at the static `git_str__initstr`
(`git_buf_fromstr` / `git_buf_sanitize`). The public header makes this unknowable from
the outside: `git2/buffer.h` documents `reserved` as "This field is reserved and
unused". So a consumer following the workset's docs, with a buffer the public API
handed back, adopts the pointer and frees a static.

`GIT_OPT_GET_TEMPLATE_PATH` after `GIT_OPT_SET_TEMPLATE_PATH("")` is such a path
(`settings.c`: `git_str_put` with `len == 0` allocates nothing). The reproducer does
exactly that and drops the `OdbDataStr`:

```
==229873==ERROR: AddressSanitizer: attempting free on address which was not malloc()-ed: 0x7025d8c373c0 in thread T0
    #0 0x64caeddb15f6 in free
    #1 0x64caedde4c3a in libgit2::libgit2::odb::sys::odb_backend::data_free crustify/rust/libgit2/libgit2/odb/sys/odb_backend.rs:100:14
    #2 0x64caedde4c13 in libgit2::libgit2::odb::sys::odb_backend::str_free crustify/rust/libgit2/libgit2/odb/sys/odb_backend.rs:303:14
    #3 0x64caedde4c7f in <libgit2::libgit2::odb::sys::odb_backend::OdbDataStrFree as ffibox::traits::CDrop<i8>>::c_drop /opt/venv/share/ffibox/src/macros.rs:367:34
    #4 0x64caedde4d61 in <ffibox::refs::CStrBox<libgit2::libgit2::odb::sys::odb_backend::OdbDataStrFree> as core::ops::drop::Drop>::drop /opt/venv/share/ffibox/src/refs.rs:689:30
    #7 0x64caedde43b0 in odb_data_str_adoption_ignores_git_buf_ownership::main .../unsafe/odb-data-str-adoption-ignores-git-buf-ownership/src/main.rs:37:5
0x7025d8c373c0 is located 0 bytes inside of global variable 'git_str__initstr' defined in '/target/src/util/str.c:15' (0x7025d8c373c0) of size 1
SUMMARY: AddressSanitizer: bad-free in free
```

The reproducer violates `from_raw`'s contract knowingly (its SAFETY comment says
so): the defect is that nothing in the workset lets a caller tell, and the docs
suggest the opposite. The batch's own equivalence test knew the rule — it asserts
`out.reserved > 0` before adopting `GIT_OPT_GET_USER_AGENT`'s buffer — but did not
surface it in the API.

## Fix

- The module docs and `OdbDataStrFree`'s docs now state the rule: a `git_strarray`
  element or `git_signature` name is always libgit2's allocation; a `git_buf`'s
  `ptr` only while `reserved > 0`, otherwise it is the static empty string that
  `git_buf_dispose` leaves alone.
- `OdbDataStrFree::take_buf(&mut ffi::git_buf) -> Option<OdbDataStr>` encodes the
  rule: it moves the string out when the buffer owns one, returns `None` when it
  does not, and leaves the buffer disposed (zeroed) either way, so a later
  `git_buf_dispose` or refill is fine. It stays `unsafe` because `ffi::git_buf` is a
  plain struct anyone can fill; its contract is the narrower, checkable "as libgit2
  left it". It is a documented raw seam over the unwrapped `git_buf`, which that
  type's batch replaces.
- Regression evidence: `tests/ub_unsafe/sys_odb_backend.rs` (`take_buf_*`: an
  owned buffer, the static empty string, a zeroed buffer, reuse after taking, a
  disposed buffer, cross-thread drop; ASan/LSan flag a bad free, double free or
  leak) and `tests/equiv/sys_odb_backend.rs` (`str_taken_from_a_git_buf`,
  `str_taken_from_an_unowned_git_buf`: traced against `git_buf_dispose` on both
  shapes, step by step). The equivalence test that previously used `from_raw` on a
  `git_buf` now goes through `take_buf`.
