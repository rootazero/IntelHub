#!/usr/bin/env bash
# build-probe-image.sh — build (once) the prebuilt probe image.
#
# The GEV motion / layer probes used to re-download chromium + its apt
# dependencies on every single run (~0.6-1.2 GB per run, ~4 GB per sp8
# run with SP8_MOTION_PROBE=1). They now run from a prebuilt image:
#
#   intelhub/probe:playwright-<version>
#
# The playwright version is derived from console/package-lock.json (the
# version the repo's own node_modules pins) so the image's browser build
# can never drift from the probe scripts.
#
# Usage:
#   bash scripts/build-probe-image.sh              # build if missing
#   PROBE_FORCE_BUILD=1 bash scripts/build-probe-image.sh
#
# Output: the image tag on stdout (last line). All logs go to stderr, so
# the caller can capture the tag with `| tail -1` without noise.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."   # repo root
DOCKERFILE="scripts/probe.Dockerfile"

log() { printf '[build-probe-image] %s\n' "$*" >&2; }

# --- playwright version pinned by the repo's lockfile --------------------
pw_version() {
  local v=""
  if command -v python3 >/dev/null 2>&1 && [[ -f console/package-lock.json ]]; then
    v=$(python3 - <<'PY' 2>/dev/null || true
import json, pathlib
try:
    lock = json.loads(pathlib.Path("console/package-lock.json").read_text())
    print(lock["packages"]["node_modules/playwright"]["version"])
except Exception:
    pass
PY
)
  fi
  if [[ -z "$v" ]]; then
    # Fallback: grep the lockfile without python.
    v=$(grep -A3 '"node_modules/playwright"' console/package-lock.json 2>/dev/null \
        | grep -m1 '"version"' | sed -E 's/.*"version": *"([^"]+)".*/\1/' || true)
  fi
  printf '%s' "$v"
}

PW_VERSION="$(pw_version)"
if [[ -z "$PW_VERSION" ]]; then
  log "ERROR: cannot determine playwright version from console/package-lock.json"
  exit 1
fi

TAG="intelhub/probe:playwright-${PW_VERSION}"

if [[ "${PROBE_FORCE_BUILD:-0}" != "1" ]] && docker image inspect "$TAG" >/dev/null 2>&1; then
  log "image $TAG already present — skipping build"
  echo "$TAG"
  exit 0
fi

log "building $TAG (playwright ${PW_VERSION}) — first build downloads ~0.6-1.2 GB, later runs are free"
docker build \
  --file "$DOCKERFILE" \
  --build-arg "PLAYWRIGHT_VERSION=${PW_VERSION}" \
  --tag "$TAG" \
  scripts/

echo "$TAG"
