/*
 * Callable shims for libgit2 function-like macros, and getters for constant macros
 * bindgen cannot evaluate, one `crustify_<NAME>` per macro.
 * Agent-owned: declare a shim here, define it in shims.c, and allowlist it in
 * functions.txt. Add a shim only for a real bindgen limitation.
 */
#ifndef CRUSTIFY_LIBGIT2_SHIMS_H
#define CRUSTIFY_LIBGIT2_SHIMS_H

#include <git2.h>

/*
 * GIT_OID_DEFAULT expands to an enum constant, which bindgen cannot evaluate as a
 * macro, so it is bound through this getter.
 */
git_oid_t crustify_GIT_OID_DEFAULT(void);

#endif
