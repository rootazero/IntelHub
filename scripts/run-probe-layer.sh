#!/usr/bin/env bash
# run-probe-layer.sh — docker wrapper for probe-layer.mjs (GEV P22).
#
# Same pattern as run-probe-motion.sh: node:22-trixie docker with --network=host
# on the hub VM (which has docker + the playwright chromium download cache);
# bind-mounts the repo so console/node_modules/playwright resolves; installs
# playwright + pngjs into /tmp/probe-deps so the host's bind-mounted
# console/node_modules/ stays untouched.
#
# Usage:
#   bash scripts/run-probe-layer.sh <layer> [base-url]
#
#   layer     aircraft | satellites | vessels | traffic (case-insensitive)
#   base-url  default http://10.10.10.35:8800  (override to test 410)
#
# Env (forwarded to the container):
#   PROBE_API_KEY        ihk_<hex> for the SPA auth gate
#   PROBE_WAIT_INITIAL   ms to settle after load (default 30000)
#   PROBE_WAIT_BETWEEN   ms between t0/t1 (default 10000)
#   PROBE_OUT            host dir for screenshots (default /tmp/probe-layer)

set -euo pipefail

LAYER="${1:-aircraft}"
LAYER="$(echo "$LAYER" | tr '[:upper:]' '[:lower:]')"
URL="${2:-${PROBE_URL:-http://10.10.10.35:8800/globe}}"
OUT="${PROBE_OUT:-/tmp/probe-layer}"
mkdir -p "$OUT"

cd "$(dirname "${BASH_SOURCE[0]}")/.."  # repo root

docker run --rm \
  --network=host \
  -v "$(pwd):/ws" \
  -v "$OUT:/out" \
  -e PROBE_URL="$URL" \
  -e PROBE_OUT=/out \
  -e PROBE_WAIT_INITIAL="${PROBE_WAIT_INITIAL:-30000}" \
  -e PROBE_WAIT_BETWEEN="${PROBE_WAIT_BETWEEN:-10000}" \
  ${PROBE_API_KEY:+-e PROBE_API_KEY="$PROBE_API_KEY"} \
  -w /ws/console \
  node:22-trixie \
  bash -c '
    set -euo pipefail
    echo "[run-probe-layer] installing playwright chromium (one-time, ~200MB)"
    npx --yes playwright install chromium --with-deps
    PROBE_DEPS=/tmp/probe-deps
    mkdir -p "$PROBE_DEPS" && cd "$PROBE_DEPS"
    if [[ ! -d node_modules/playwright ]]; then
      npm init -y >/dev/null 2>&1
      npm install --silent --no-audit --no-fund playwright pngjs >/dev/null 2>&1
    fi
    cp /ws/console/probe-layer.mjs "$PROBE_DEPS/probe-layer.mjs"
    cd "$PROBE_DEPS"
    echo "[run-probe-layer] running probe layer='"$LAYER"'"
    node ./probe-layer.mjs --url="'"$URL"'" --layer='"$LAYER"' --out=/out
  '
