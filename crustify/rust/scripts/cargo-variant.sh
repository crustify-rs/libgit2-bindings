#!/bin/bash
# Run cargo against one crustify prebuild of libgit2, instrumenting Rust to match.
#
#   crustify/rust/scripts/cargo-variant.sh <variant> <cargo subcommand> [args...]
#
# <variant> is a `variants` key of crustify/build.json: plain, coverage, asan-ubsan,
# tsan or bsan. Each variant uses its own target dir under crustify/rust/target/.
# Set CRUSTIFY_LIBGIT2_BUILD_DIR to link a private build instead of the shared one.
set -euo pipefail
variant=${1:?usage: cargo-variant.sh <variant> <cargo subcommand> [args...]}
shift
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
host=x86_64-unknown-linux-gnu
export CRUSTIFY_BUILD_VARIANT=$variant
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$here/target/$variant}
cd "$here"
case $variant in
  plain)
    exec cargo "$@" ;;
  coverage)
    # Rust coverage via cargo-llvm-cov; the C side writes gcov data into the coverage build.
    exec cargo llvm-cov "$@" ;;
  asan-ubsan)
    # Instrument only the target triple. A global RUSTFLAGS also reaches host build
    # scripts and proc-macros, and nightly rejects building a proc-macro with
    # sanitizers ("building proc macro crate with sanitizers enabled is not
    # supported"); dev-dependencies such as trybuild pull one in (serde_derive). The
    # per-target override leaves host artifacts uninstrumented while every crate built
    # for $host -- the lib, its tests and their dependencies, where libgit2's
    # allocations live -- is still sanitized, so LSan still sees leaks.
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=address -Cforce-frame-pointers=yes"
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -Zsanitizer=address"
    export ASAN_OPTIONS=${ASAN_OPTIONS:-detect_leaks=1:abort_on_error=1}
    export UBSAN_OPTIONS=${UBSAN_OPTIONS:-print_stacktrace=1:halt_on_error=1}
    exec cargo +nightly "$@" --target $host ;;
  tsan)
    # Per-target, like asan-ubsan above: a global RUSTFLAGS reaches host proc-macros,
    # which nightly refuses to build with sanitizers (dev-deps pull serde_derive).
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=thread"
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -Zsanitizer=thread"
    export TSAN_OPTIONS=${TSAN_OPTIONS:-halt_on_error=1}
    # trybuild's nested cargo cannot build the sanitized dependencies, and the
    # compile-fail suite is a compile-time check the plain variant already runs; skip
    # it, like the bsan branch. Only for a plain `test` with no `-- <args>` of its own,
    # since the skip and the appended --target share the one `--` separator.
    if [ "${1:-}" = test ] && [[ " $* " != *" -- "* ]]; then
      exec cargo +nightly "$@" -Zbuild-std --target $host -- --skip compile_fail
    fi
    exec cargo +nightly "$@" -Zbuild-std --target $host ;;
  bsan)
    # trybuild's nested cargo build does not finish under BorrowSanitizer, and the
    # compile-fail suite is a compile-time check the plain variant already runs.
    if [ "${1:-}" = test ]; then
      case " $* " in
        *" -- "*) exec cargo bsan "$@" --skip compile_fail ;;
        *) exec cargo bsan "$@" -- --skip compile_fail ;;
      esac
    fi
    exec cargo bsan "$@" ;;
  *)
    echo "unknown variant: $variant" >&2; exit 2 ;;
esac
