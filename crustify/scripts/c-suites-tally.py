"""Tally a c-suites.sh run: case counts and every suite that failed or exited non-zero."""
import glob, json, os, re, sys

d = sys.argv[1]
ok = notok = skip = 0
bad = []
for rc in sorted(glob.glob(f"{d}/suites/*/*.rc")):
    base = rc[:-3]
    tap = open(base + ".tap", errors="replace").read()
    sk = len(re.findall(r"^ok \d+ .*# SKIP", tap, re.M))
    ok += len(re.findall(r"^ok \d+", tap, re.M)) - sk
    skip += sk
    failed = re.findall(r"^not ok \d+ - (\S+)", tap, re.M)
    notok += len(failed)
    code = open(rc).read().strip()
    if code != "0" or failed:
        err = open(base + ".err", errors="replace").read()
        sig = sorted(set(re.findall(r"ThreadSanitizer: [\w\- ]+?(?= \(|$)|error: Undefined Behavior"
                                    r"|AddressSanitizer: \S+|runtime error", err, re.M)))
        bad.append({"binary": os.path.basename(os.path.dirname(base)),
                    "suite": os.path.basename(base), "exit": int(code),
                    "failed_cases": failed, "signals": sig})
print(json.dumps({"passed": ok, "failed": notok, "skipped": skip,
                  "total": ok + notok + skip, "nonzero_suites": bad}, indent=1))
