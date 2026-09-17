#!/usr/bin/env bash
# GEV-engine sync governance check — the scripted half of the T12 contract
# guard (the vitest half lives in
# console/src/gev-boot/__tests__/source-contracts.test.ts).
#
# Local and deterministic; the only network call (upstream HEAD vs pinned SHA)
# is best-effort and never affects the exit code. Run after any vendor sync and
# in CI before a sync is committed.
#
# Checks:
#   1. UPSTREAM.json parses and carries the expected schema (pinned_sha /
#      excludes / exceptions).
#   2. The vendored tree is clean against git — no local edits or stray files
#      under console/gev-engine. UPSTREAM.json itself is the seam file and is
#      exempt (it is IntelHub-owned by design and excluded from rsync).
#      A dirty vendored tree means someone hand-patched the vendor: that diff
#      is exactly what this guard exists to catch.
#   3. Every file listed in UPSTREAM.json exceptions exists on disk.
#   4. (WARN only, needs network) upstream HEAD vs pinned_sha — upstream
#      movement is the governed-upgrade trigger, not a failure.
#
# Usage: bash scripts/check-gev-engine-sync.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR_REL="console/gev-engine"
META="$ROOT/$VENDOR_REL/UPSTREAM.json"

fail() { echo "check-gev-engine-sync: FAIL: $*" >&2; exit 1; }

# ── 1. UPSTREAM.json schema ────────────────────────────────────────────────
python3 - "$META" <<'PY' || fail "UPSTREAM.json missing/invalid schema"
import json, re, sys
meta = json.load(open(sys.argv[1]))
sha = meta.get("pinned_sha", "")
assert re.fullmatch(r"[0-9a-f]{40}", sha), f"pinned_sha not a full sha: {sha!r}"
assert isinstance(meta.get("excludes"), list), "excludes must be a list"
exc = meta.get("exceptions", [])
assert isinstance(exc, list), "exceptions must be a list"
for e in exc:
    f = e if isinstance(e, str) else e.get("file")
    assert isinstance(f, str) and f, f"exception entry without a file: {e!r}"
print(f"UPSTREAM.json ok: pinned {sha[:12]}, {len(exc)} exception(s)")
PY

# ── 2. vendored tree clean vs git (UPSTREAM.json seam exempt) ──────────────
cd "$ROOT"
DIRTY=$(git status --porcelain -- "$VENDOR_REL" ":(exclude)$VENDOR_REL/UPSTREAM.json")
[ -z "$DIRTY" ] || fail "vendored tree has local modifications (governed upgrades only — revert or re-sync):\n$DIRTY"
echo "vendored tree clean vs git (UPSTREAM.json seam exempt)"

# ── 3. declared exception files exist ──────────────────────────────────────
while IFS= read -r f; do
  [ -e "$ROOT/$f" ] || fail "declared exception file missing on disk: $f"
done < <(python3 -c "import json;[print(e if isinstance(e,str) else e['file']) for e in json.load(open('$META')).get('exceptions',[])]")
echo "all declared exception files exist"

# ── 4. upstream HEAD vs pinned (best-effort, WARN only) ────────────────────
REPO=$(python3 -c "import json;print(json.load(open('$META'))['repo'])")
PINNED=$(python3 -c "import json;print(json.load(open('$META'))['pinned_sha'])")
REMOTE=$(git -c http.lowSpeedLimit=1 -c http.lowSpeedTime=10 ls-remote "$REPO" HEAD 2>/dev/null | cut -f1 || true)
if [ -z "$REMOTE" ]; then
  echo "WARN: could not reach upstream (offline?) — skipped HEAD comparison"
elif [ "$REMOTE" != "$PINNED" ]; then
  echo "WARN: upstream moved HEAD=$REMOTE pinned=$PINNED — run scripts/sync-gev-engine.sh for a governed upgrade, then the c1 contract guard"
else
  echo "upstream HEAD matches pinned $PINNED"
fi

echo "check-gev-engine-sync: OK"
