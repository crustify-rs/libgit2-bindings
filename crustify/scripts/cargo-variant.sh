#!/bin/bash
# Run cargo against one crustify prebuild of libgit2, instrumenting Rust to match.
#
#   crustify/scripts/cargo-variant.sh <variant> <cargo subcommand> [args...]
#
# <variant> is a `variants` key of crustify/artifacts/build.json: plain, coverage, asan-ubsan,
# tsan or bsan. Each variant uses its own target dir under target/.
# Set CRUSTIFY_LIBGIT2_BUILD_DIR to link a private build instead of the shared one.
set -euo pipefail
variant=${1:?usage: cargo-variant.sh <variant> <cargo subcommand> [args...]}
shift
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
host=x86_64-unknown-linux-gnu
export CRUSTIFY_BUILD_VARIANT=$variant
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$here/target/$variant}
cd "$here"
# The nightly the workspace pins, for the sanitizers that need it.
nightly=$(sed -n 's/^nightly = "\(.*\)"$/\1/p' Cargo.toml)
# Cargo's own flags must come before a `--`, which hands the rest to the test binaries.
pre=() post=()
for arg in "$@"; do
  if [ ${#post[@]} -gt 0 ] || [ "$arg" = -- ]; then post+=("$arg"); else pre+=("$arg"); fi
done
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
    exec cargo "+$nightly" "${pre[@]}" --target $host "${post[@]}" ;;
  tsan)
    export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=thread"
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -Zsanitizer=thread"
    export TSAN_OPTIONS=${TSAN_OPTIONS:-halt_on_error=1}
    exec cargo "+$nightly" "${pre[@]}" -Zbuild-std --target $host "${post[@]}" ;;
  bsan)
    # trybuild's nested cargo build does not finish under BorrowSanitizer, and the
    # compile-fail suite is a compile-time check the plain variant already runs.
    # Under parallel test threads BorrowSanitizer's GC can take longer than its default
    # 10 s hang threshold to bring every thread to a safepoint, and then aborts; give it
    # 120 s rather than serialize the tests.
    export BSAN_OPTIONS=${BSAN_OPTIONS:-gc_hang_threshold_ms=120000}
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
