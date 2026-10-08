/*
 * Callable shims for libgit2 function-like macros, one `crustify_<NAME>` per macro.
 * Agent-owned: declare a shim here, define it in shims.c, and allowlist it in
 * functions.txt. Add a shim only for a real bindgen limitation.
 */
#ifndef CRUSTIFY_LIBGIT2_SHIMS_H
#define CRUSTIFY_LIBGIT2_SHIMS_H

#include <git2.h>

#endif
