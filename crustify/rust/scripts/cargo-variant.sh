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
    export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=address -Cforce-frame-pointers=yes"
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -Zsanitizer=address"
    export ASAN_OPTIONS=${ASAN_OPTIONS:-detect_leaks=1:abort_on_error=1}
    export UBSAN_OPTIONS=${UBSAN_OPTIONS:-print_stacktrace=1:halt_on_error=1}
    exec cargo +nightly "$@" --target $host ;;
  tsan)
    export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=thread"
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -Zsanitizer=thread"
    export TSAN_OPTIONS=${TSAN_OPTIONS:-halt_on_error=1}
    exec cargo +nightly "$@" -Zbuild-std --target $host ;;
  bsan)
    # trybuild's nested cargo build does not finish under BorrowSanitizer, and the
    # compile-fail suite is a compile-time check the plain variant already runs.
    # Tests run on one thread: BorrowSanitizer's garbage collector can wait forever for
    # an exited test thread to reach a safepoint when many threads churn at once.
    if [ "${1:-}" = test ]; then
      case " $* " in
        *" -- "*) exec cargo bsan "$@" --skip compile_fail --test-threads=1 ;;
        *) exec cargo bsan "$@" -- --skip compile_fail --test-threads=1 ;;
      esac
    fi
    exec cargo bsan "$@" ;;
  *)
    echo "unknown variant: $variant" >&2; exit 2 ;;
esac
