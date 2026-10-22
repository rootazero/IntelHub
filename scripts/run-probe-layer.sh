#!/usr/bin/env bash
# run-probe-layer.sh — docker wrapper for probe-layer.mjs (GEV P22).
#
# Runs the headless multi-layer motion probe from a PREBUILT image
# (intelhub/probe:playwright-<version>, built by scripts/build-probe-image.sh).
#
# Why the prebuilt image (2026-10-23, "36 GB incident"):
#   * The old wrapper ran `npx --yes playwright install chromium --with-deps`
#     inside a throwaway `node:22-trixie` container on EVERY run. The
#     container was removed afterwards, so the apt packages + chromium +
#     headless-shell + ffmpeg (~0.6-1.2 GB) were re-downloaded every time.
#     One sp8 run with SP8_MOTION_PROBE=1 (1 motion + 3 layer probes)
#     pulled ~4 GB off the WAN.
#   * The old wrapper mounted the WHOLE repo read-write at /ws, so the
#     container — which executes third-party npm code — could read and
#     rewrite core/secrets.env, core/hub.env, agent-keys.txt, ...
#     Now only the single .mjs file is mounted, read-only: /ws no longer
#     exists inside the container.
#   * The old wrapper used `--network=host`, putting the container in the
#     host's network namespace. We tried to flip the default to the
#     docker bridge, but that cannot work on this deployment: the host
#     firewall (scripts/bootstrap-host.sh) is `table inet filter` with
#     `input` policy DROP and a single allow rule for LAN sources
#     (`ip saddr 10.10.10.0/24 tcp dport { 22,3000,5001,8080,8800,11235 }`).
#     A bridge container's source is 172.x, so *every* packet it sends to
#     the host — including the hub's own LAN listener 10.10.10.45:8800 —
#     is dropped (verified: bridge curl to the hub times out after 8s,
#     host-network curl answers in 1.5ms). Host networking is therefore
#     the default again; the container's blast radius against the host is
#     capped by the same allow-list any LAN host has. Set
#     PROBE_NETWORK=bridge on hosts whose firewall permits it.
#
# Usage:
#   bash scripts/run-probe-layer.sh <layer> [base-url]
#
#   layer     aircraft | satellites | vessels | traffic (case-insensitive)
#   base-url  default http://<this host's own LAN IP>:8800/globe
#              (the probe runs on the hub VM itself, so it must target
#               *this* VM — a hardcoded peer IP silently screenshots the
#               other VM's globe; the old default 10.10.10.35 is dead)
#
# Env (forwarded to the container):
#   PROBE_API_KEY        ihk_<hex> for the SPA auth gate
#   PROBE_WAIT_INITIAL   ms to settle after load (default 30000)
#   PROBE_WAIT_BETWEEN   ms between t0/t1 (default 10000)
#   PROBE_OUT            host dir for screenshots (default /tmp/probe-layer)
#   PROBE_NETWORK        docker network mode (default: host — see above)
#
# The first invocation on a fresh VM builds the image (~0.6-1.2 GB, a few
# minutes). Build it ahead of time with:
#   bash scripts/build-probe-image.sh
set -euo pipefail

LAYER="${1:-aircraft}"
LAYER="$(echo "$LAYER" | tr '[:upper:]' '[:lower:]')"

# LAN IP of this host (source address of the default route). Falls back to
# loopback when the route lookup is unavailable.
local_ip() {
  ip -4 route get 1.1.1.1 2>/dev/null \
    | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit }}'
}
LOCAL_IP="$(local_ip || true)"
URL="${2:-${PROBE_URL:-http://${LOCAL_IP:-127.0.0.1}:8800/globe}}"
OUT="${PROBE_OUT:-/tmp/probe-layer}"
mkdir -p "$OUT"

cd "$(dirname "${BASH_SOURCE[0]}")/.."  # repo root

SCRIPT_SRC="$(pwd)/console/probe-layer.mjs"
if [[ ! -f "$SCRIPT_SRC" ]]; then
  echo "[run-probe-layer] ERROR: $SCRIPT_SRC not found" >&2
  exit 1
fi

# --- prebuilt image (build once, then free) ------------------------------
BUILD_LOG="${PROBE_IMAGE_BUILD_LOG:-/tmp/probe-image-build.log}"
TAG_FILE="$(mktemp)"
if ! bash scripts/build-probe-image.sh >"$TAG_FILE" 2>"$BUILD_LOG"; then
  echo "[run-probe-layer] ERROR: probe image build failed — see $BUILD_LOG" >&2
  tail -n 20 "$BUILD_LOG" >&2 || true
  rm -f "$TAG_FILE"
  exit 1
fi
IMAGE="$(tail -n 1 "$TAG_FILE")"
rm -f "$TAG_FILE"
echo "[run-probe-layer] image=$IMAGE network=${PROBE_NETWORK:-host}" >&2

ENV_ARGS=()
if [[ -n "${PROBE_API_KEY:-}" ]]; then
  ENV_ARGS+=(-e "PROBE_API_KEY=$PROBE_API_KEY")
fi

docker run --rm \
  --network="${PROBE_NETWORK:-host}" \
  -v "$SCRIPT_SRC:/opt/probe/probe-layer.mjs:ro" \
  -v "$OUT:/out" \
  -e PROBE_URL="$URL" \
  -e PROBE_OUT=/out \
  -e PROBE_WAIT_INITIAL="${PROBE_WAIT_INITIAL:-30000}" \
  -e PROBE_WAIT_BETWEEN="${PROBE_WAIT_BETWEEN:-10000}" \
  "${ENV_ARGS[@]+"${ENV_ARGS[@]}"}" \
  -w /opt/probe \
  "$IMAGE" \
  node ./probe-layer.mjs --url="$URL" --layer="$LAYER" --out=/out
