#!/bin/bash
# baseline.sh <crate checkout> <package> <cov dir> <nightly> [cargo test args...]: as run.sh, for a
# comparison crate: its own C built by its build script, instrumented through CC/CFLAGS.
set -uo pipefail
trap 'echo "failed at line $LINENO" >&2' ERR
src=$1 pkg=$2 out=$3 nightly=$4; shift 4
host=x86_64-unknown-linux-gnu
tree=$out/tree
rm -rf "$out"; mkdir -p "$tree" "$out/prof"
(cd "$src" && tar -cf - --exclude=./target --exclude=./.git .) | (cd "$tree" && tar -xf -)
# a scratch copy inside another workspace's tree is its own workspace
grep -q '^\[workspace\]' "$tree/Cargo.toml" || printf '\n[workspace]\n' >> "$tree/Cargo.toml"
root=$(cd "$tree" && cargo metadata --no-deps --format-version 1 | python3 -c "import sys,json;d=json.load(sys.stdin);p=[x for x in d['packages'] if x['name']=='$pkg'][0];print([t['src_path'] for t in p['targets'] if 'lib' in t['kind']][0])")
python3 "$(dirname "$0")/mark.py" "$(dirname "$root")" "$root"
cd "$(dirname "$root")/.."
export CARGO_TARGET_DIR=$out/target CC=/usr/lib/llvm-23/bin/clang CFLAGS="-fprofile-instr-generate -fcoverage-mapping"
export RUSTFLAGS="-C instrument-coverage" RUSTDOCFLAGS="-C instrument-coverage -Z unstable-options --persist-doctests $out/doctests"
cargo "+$nightly" test -p "$pkg" "$@" --target $host --no-run --message-format=json 2>"$out/build.err" \
  | python3 -c 'import sys,json
for l in sys.stdin:
  try: d=json.loads(l)
  except: continue
  if d.get("reason")=="compiler-artifact" and d.get("executable") and d["target"]["kind"]!=["custom-build"] and d.get("profile",{}).get("test"): print(d["target"]["name"],d["target"]["kind"][0],d["executable"])' > "$out/bins.txt"
cat "$out/bins.txt"; tail -2 "$out/build.err"
while read -r name kind exe; do
  suite=$([ "$kind" = lib ] && echo lib || echo "$name")
  mkdir -p "$out/prof/$suite"; echo "== $suite"
  LLVM_PROFILE_FILE="$out/prof/$suite/%p-%m.profraw" "$exe" > "$out/$suite.log" 2>&1 || echo "rc=$? in $suite"
  grep 'test result' "$out/$suite.log"
  "$exe" --list 2>/dev/null | grep -c ': test$' > "$out/$suite.count"
done < "$out/bins.txt"
mkdir -p "$out/prof/doc"; echo "== doc"
LLVM_PROFILE_FILE="$out/prof/doc/%p-%m.profraw" cargo "+$nightly" test -p "$pkg" "$@" --target $host --doc > "$out/doc.log" 2>&1 || echo "rc=$? in doc"
grep 'test result' "$out/doc.log"
