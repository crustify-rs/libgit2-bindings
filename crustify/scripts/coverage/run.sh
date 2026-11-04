#!/bin/bash
# run.sh <repo> <package> <cov dir>: build a coverage-instrumented scratch copy of the safe
# crate against the `coverage` variant, its test items under #[coverage(off)], and run each
# suite alone into <cov dir>/prof/<suite>: lib, each integration test binary, and doc.
# report.py then reads the figures (testing spec, Coverage).
set -euo pipefail
repo=$1 pkg=$2 out=$3
host=x86_64-unknown-linux-gnu
nightly=$(sed -n 's/^nightly = "\(.*\)"$/\1/p' "$repo/Cargo.toml")
tree=$out/tree
rm -rf "$out"; mkdir -p "$tree/crustify" "$out/prof"
(cd "$repo" && tar -cf - --exclude=./target --exclude=./crustify --exclude=./c --exclude=./.git .) | (cd "$tree" && tar -xf -)
ln -s "$repo/c" "$tree/c"
ln -s "$repo/crustify/artifacts" "$tree/crustify/artifacts"
ln -s "$repo/crustify/builds" "$tree/crustify/builds"
python3 "$(dirname "$0")/mark.py" "$tree/src" "$tree/src/lib.rs"
cd "$tree"
export CRUSTIFY_BUILD_VARIANT=coverage CARGO_TARGET_DIR=$out/target
export RUSTFLAGS="-C instrument-coverage" RUSTDOCFLAGS="-C instrument-coverage -Z unstable-options --persist-doctests $out/doctests"
cargo "+$nightly" test -p "$pkg" --target $host --no-run --message-format=json 2>"$out/build.err" \
  | python3 -c 'import sys,json
for l in sys.stdin:
  try: d=json.loads(l)
  except: continue
  if d.get("reason")=="compiler-artifact" and d.get("executable") and d["target"]["kind"]!=["custom-build"] and d.get("profile",{}).get("test"): print(d["target"]["name"],d["target"]["kind"][0],d["executable"])' > "$out/bins.txt"
cat "$out/bins.txt"
while read -r name kind exe; do
  suite=$([ "$kind" = lib ] && echo lib || echo "$name")
  mkdir -p "$out/prof/$suite"
  echo "== $suite"
  LLVM_PROFILE_FILE="$out/prof/$suite/%p-%m.profraw" "$exe" --skip compile_fail > "$out/$suite.log" 2>&1 || echo "rc=$? in $suite"
  tail -3 "$out/$suite.log"
  "$exe" --list 2>/dev/null | grep -c ': test$' > "$out/$suite.count" || true
done < "$out/bins.txt"
mkdir -p "$out/prof/doc"
echo "== doc"
LLVM_PROFILE_FILE="$out/prof/doc/%p-%m.profraw" cargo "+$nightly" test -p "$pkg" --target $host --doc > "$out/doc.log" 2>&1 || echo "rc=$? in doc"
tail -3 "$out/doc.log"
