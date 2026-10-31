#!/usr/bin/env bash
# Runs the bundled replay against a running engine and checks that the catch still happens:
# tests pass before and after the recorded patch, the behaviour probe finds changed behaviour, and the engine reverts.
# Exits non-zero otherwise. Used by CI (.github/workflows/ci.yml) and runnable by hand.
#
#   scripts/replay-check.sh [engine-url] [record-output-path]
set -euo pipefail
ENGINE="${1:-http://127.0.0.1:7878}"
OUT="${2:-}"

id=$(curl -fsS -X POST "$ENGINE/api/replays" -H 'Content-Type: application/json' -d '{}' |
  python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
echo "replay inheritance: $id"

for _ in $(seq 1 360); do
  active=$(curl -fsS "$ENGINE/api/inheritances/$id" | python3 -c 'import json,sys; print(json.load(sys.stdin)["active_job"] is not None)')
  [ "$active" = "False" ] && break
  sleep 2
done

curl -fsS "$ENGINE/api/inheritances/$id" | python3 -c '
import json, re, sys
d = json.load(sys.stdin)
job = d["jobs"][-1]
steps = {s["name"]: s for s in job["steps"]}
for s in job["steps"]:
    print("  [%6s] %s: %s" % (s["status"], s["name"], s.get("detail") or ""))
def passed(name):
    m = re.search(r"(\d+) passed, (\d+) failed", steps[name]["detail"] or "")
    return (int(m.group(1)), int(m.group(2))) if m else None
repair = d["repairs"][-1]
checks = {
    "job completed": job["status"] == "completed",
    "tests before: 126 passed, 0 failed": passed("tests before") == (126, 0),
    "tests after: 126 passed, 0 failed": passed("tests after") == (126, 0),
    "probe verdict changed": (repair.get("probe") or {}).get("verdict") == "changed",
    "repair reverted": repair["status"] == "reverted",
    "recorded patch never committed": not repair.get("commit"),
}
for k, ok in checks.items():
    print(("PASS " if ok else "FAIL ") + k)
sys.exit(0 if all(checks.values()) else 1)
'

if [ -n "$OUT" ]; then
  curl -fsS "$ENGINE/api/inheritances/$id/record" -o "$OUT"
  echo "inheritance record written to $OUT"
fi
