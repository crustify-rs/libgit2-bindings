#!/bin/bash
# Run every offline libgit2 clar suite and every util suite of one prebuild in isolation,
# in parallel, then tally the cases against the campaign baseline.
#
#   crustify/scripts/c-suites.sh <build dir> [jobs]
#
# Results land in <build dir>/suites/<binary>/<suite>.{tap,err,rc}. `online::*` suites
# need the network and are excluded, as the `offline` ctest target does.
set -uo pipefail
d=$(cd "${1:?usage: c-suites.sh <build dir> [jobs]}" && pwd)
jobs=${2:-28}
mkdir -p "$d/suites"
for b in libgit2_tests util_tests; do
  mkdir -p "$d/suites/$b"
  "$d/$b" -l 2>/dev/null | sed -nE 's/^ *[0-9]+: (.*)$/\1/p' | grep -v '^online::' |
    D="$d" B="$b" xargs -P "$jobs" -I{} bash -c \
      'cd "$D" && timeout 1800 "./$B" -t "-s$1" > "suites/$B/$1.tap" 2> "suites/$B/$1.err"; echo $? > "suites/$B/$1.rc"' _ {}
done
python3 -I "$(dirname "${BASH_SOURCE[0]}")/c-suites-tally.py" "$d"
