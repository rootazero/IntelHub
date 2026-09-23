#!/usr/bin/env bash
# run-probe-motion.sh — run console/probe-motion.mjs inside the node:22-trixie
# docker container with the hub repo bind-mounted.
#
# Why a wrapper (GEV P21 T1):
#   * Host 315 has no node in PATH and no chromium system libs.
#   * `console/node_modules/playwright` (used by `probe-gev.mjs`) is already
#     installed in the repo tree; mounting the repo into the container makes
#     that wrapper resolvable without re-installing.
#   * `npx playwright install chromium --with-deps` downloads the browser
#     (~200 MB, one-time) into the container's own cache layer so nothing
#     leaks onto the host.
#   * `--network=host` is required because the probe hits the hub's LAN IP
#     (10.10.10.35:8800) — docker's default bridge network would isolate it.
#
# Usage:
#   bash scripts/run-probe-motion.sh [URL]                  # default: globe on 315
#   PROBE_OUT=/some/path bash scripts/run-probe-motion.sh    # override screenshot dir
#
# Exit codes mirror probe-motion.mjs:
#   0  motion verified (≥0.5% pixel diff)
#   1  static (byte-identical)
#   2  essentially static (<0.05%)
#   3  minimal motion (<0.5%)
set -euo pipefail

URL="${1:-${PROBE_URL:-http://10.10.10.35:8800/}}"
OUT="${PROBE_OUT:-/tmp/probe-motion}"
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
    echo "[run-probe-motion] installing playwright chromium (one-time, ~200MB)"
    npx --yes playwright install chromium --with-deps
    # Install playwright + pngjs in /tmp so the bind-mounted
    # /ws/console/node_modules/ stays untouched on the host. probe-motion.mjs
    # runs from /tmp/probe-deps so module resolution finds both packages.
    PROBE_DEPS=/tmp/probe-deps
    mkdir -p "$PROBE_DEPS" && cd "$PROBE_DEPS"
    if [[ ! -d node_modules/playwright ]]; then
      npm init -y >/dev/null 2>&1
      npm install --silent --no-audit --no-fund playwright pngjs >/dev/null 2>&1
    fi
    cp /ws/console/probe-motion.mjs "$PROBE_DEPS/probe-motion.mjs"
    cd "$PROBE_DEPS"
    echo "[run-probe-motion] running probe"
    node ./probe-motion.mjs
  '