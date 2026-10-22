#!/usr/bin/env bash
# run-probe-motion.sh — run console/probe-motion.mjs inside the prebuilt
# probe image (intelhub/probe:playwright-<version>).
#
# Why a wrapper (GEV P21 T1) and why a prebuilt image:
#   * The hub VM has no node in PATH and no chromium system libs, so the
#     probe must run inside a container.
#   * The old wrapper ran `npx playwright install chromium --with-deps`
#     inside a throwaway node:22-trixie container on EVERY run — the
#     container was removed afterwards, so ~0.6-1.2 GB of apt packages +
#     chromium were re-downloaded every time (one sp8 run with
#     SP8_MOTION_PROBE=1 pulled ~4 GB). The image bakes that once.
#   * The old wrapper mounted the WHOLE repo read-write at /ws, exposing
#     core/secrets.env (and every other runtime secret) to third-party
#     npm code running as root. Now only the single .mjs file is mounted,
#     read-only.
#   * `--network=host` stays the default: the host firewall
#     (scripts/bootstrap-host.sh) is `table inet filter` with `input`
#     policy DROP and a LAN-source-only allow rule, so a bridge
#     container cannot reach the hub's LAN listener 10.10.10.45:8800 at
#     all (verified: bridge curl times out, host curl answers). Set
#     PROBE_NETWORK=bridge on hosts whose firewall permits it.
#
# Usage:
#   bash scripts/run-probe-motion.sh [URL]                  # default: this host's own globe
#   PROBE_OUT=/some/path bash scripts/run-probe-motion.sh    # override screenshot dir
#
# Env:
#   PROBE_API_KEY      ihk_<hex> for the SPA auth gate
#   PROBE_NETWORK      docker network mode (default: host)
#
# Exit codes mirror probe-motion.mjs:
#   0  motion verified (≥0.5% pixel diff)
#   1  static (byte-identical)
#   2  essentially static (<0.05%)
#   3  minimal motion (<0.5%)
set -euo pipefail

# LAN IP of this host (source address of the default route) — the probe
# runs on the hub VM itself, so it must target *this* VM. A hardcoded peer
# IP silently screenshots the other VM's globe (the old default
# 10.10.10.35 is dead).
local_ip() {
  ip -4 route get 1.1.1.1 2>/dev/null \
    | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit }}'
}
LOCAL_IP="$(local_ip || true)"
URL="${1:-${PROBE_URL:-http://${LOCAL_IP:-127.0.0.1}:8800/}}"
OUT="${PROBE_OUT:-/tmp/probe-motion}"
mkdir -p "$OUT"

cd "$(dirname "${BASH_SOURCE[0]}")/.."  # repo root

SCRIPT_SRC="$(pwd)/console/probe-motion.mjs"
if [[ ! -f "$SCRIPT_SRC" ]]; then
  echo "[run-probe-motion] ERROR: $SCRIPT_SRC not found" >&2
  exit 1
fi

# --- prebuilt image (build once, then free) ------------------------------
BUILD_LOG="${PROBE_IMAGE_BUILD_LOG:-/tmp/probe-image-build.log}"
TAG_FILE="$(mktemp)"
if ! bash scripts/build-probe-image.sh >"$TAG_FILE" 2>"$BUILD_LOG"; then
  echo "[run-probe-motion] ERROR: probe image build failed — see $BUILD_LOG" >&2
  tail -n 20 "$BUILD_LOG" >&2 || true
  rm -f "$TAG_FILE"
  exit 1
fi
IMAGE="$(tail -n 1 "$TAG_FILE")"
rm -f "$TAG_FILE"
echo "[run-probe-motion] image=$IMAGE network=${PROBE_NETWORK:-host}" >&2

ENV_ARGS=()
if [[ -n "${PROBE_API_KEY:-}" ]]; then
  ENV_ARGS+=(-e "PROBE_API_KEY=$PROBE_API_KEY")
fi

docker run --rm \
  --network="${PROBE_NETWORK:-host}" \
  -v "$SCRIPT_SRC:/opt/probe/probe-motion.mjs:ro" \
  -v "$OUT:/out" \
  -e PROBE_URL="$URL" \
  -e PROBE_OUT=/out \
  -e PROBE_WAIT_INITIAL="${PROBE_WAIT_INITIAL:-30000}" \
  -e PROBE_WAIT_BETWEEN="${PROBE_WAIT_BETWEEN:-10000}" \
  "${ENV_ARGS[@]+"${ENV_ARGS[@]}"}" \
  -w /opt/probe \
  "$IMAGE" \
  node ./probe-motion.mjs
