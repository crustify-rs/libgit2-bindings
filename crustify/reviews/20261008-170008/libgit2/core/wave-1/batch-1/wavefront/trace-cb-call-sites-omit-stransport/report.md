# `git_trace_cb` finding named httpclient.c as the only `git_trace` caller

- item: `git_trace_cb` (callback, `include/git2/trace.h`)
- record: `crustify/wavefront/ownership-store.json`, `ptr_args[1].note`
- affected commit: `2b63f91ae` (libgit2/core wave 1 batch 1), reviewed at `4dfa6b86a`
- resubmitted: this review's `crustify/reviews/` commit (`query symbols --name
  git_trace_cb --update`)

## Finding

The note submitted by the batch read "Sole invoker git_trace__write_fmt
(src/libgit2/trace.h, via git_trace from httpclient.c only)" and "always a named
GIT_TRACE_* constant at every call site (FATAL..TRACE, never NONE)". The first claim
is incomplete and the second overstates what the call sites use:

```
$ grep -rn "git_trace(" src/ --include=*.c --include=*.h | grep -v "^src/libgit2/trace.h"
src/libgit2/streams/stransport.c:269:   git_trace(GIT_TRACE_INFO, "SecureTraceport error during SSLRead: ...");
src/libgit2/transports/httpclient.c:428:  git_trace(GIT_TRACE_TRACE,
src/libgit2/transports/httpclient.c:439:  git_trace(GIT_TRACE_TRACE,
src/libgit2/transports/httpclient.c:989:    git_trace(GIT_TRACE_DEBUG, "Connecting to proxy %s port %s",
src/libgit2/transports/httpclient.c:1109: git_trace(GIT_TRACE_DEBUG, "Connecting to remote %s port %s",
src/libgit2/transports/httpclient.c:1158:   git_trace(GIT_TRACE_TRACE, "Received:\n%.*s",
src/libgit2/transports/httpclient.c:1308:   git_trace(GIT_TRACE_DEBUG, "Sending %s request to %s",
```

`streams/stransport.c` also traces (one `GIT_TRACE_INFO` message on an `SSLRead`
error). It is compiled only under `GIT_HTTPS_SECURETRANSPORT`, which the campaign's
`wavefront-config.json` lists as out of scope, so the batch's wrapper and tests were
right for every campaign build; the record, which is build-independent, was not. The
levels actually passed are INFO, DEBUG and TRACE, never FATAL, ERROR or WARN.

## Resubmission

The note now lists both files, the levels each uses, and that the stransport site is
out of the campaign's scope. The pointer facts (borrowed for the call, never NULL,
NUL-terminated, arbitrary bytes, called concurrently from the tracing threads) were
re-verified against `git_trace__write_fmt` and are unchanged.
