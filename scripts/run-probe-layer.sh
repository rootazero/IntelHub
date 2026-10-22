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
#     host's network namespace (it could reach 127.0.0.1 services such as
#     redis/postgres/neo4j admin and bind host ports). The default is now
#     the docker bridge; the probe only needs the hub's LAN URL, which is
#     reachable from the bridge. Override with PROBE_NETWORK=host if a
#     future setup needs host networking.
#
# Usage:
#   bash scripts/run-probe-layer.sh <layer> [base-url]
#
#   layer     aircraft | satellites | vessels | traffic (case-insensitive)
#   base-url  default http://10.10.10.45:8800/globe  (override to test 410)
#
# Env (forwarded to the container):
#   PROBE_API_KEY        ihk_<hex> for the SPA auth gate
#   PROBE_WAIT_INITIAL   ms to settle after load (default 30000)
#   PROBE_WAIT_BETWEEN   ms between t0/t1 (default 10000)
#   PROBE_OUT            host dir for screenshots (default /tmp/probe-layer)
#   PROBE_NETWORK        docker network mode (default: bridge)
#
# The first invocation on a fresh VM builds the image (~0.6-1.2 GB, a few
# minutes). Build it ahead of time with:
#   bash scripts/build-probe-image.sh
set -euo pipefail

LAYER="${1:-aircraft}"
LAYER="$(echo "$LAYER" | tr '[:upper:]' '[:lower:]')"
URL="${2:-${PROBE_URL:-http://10.10.10.45:8800/globe}}"
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
echo "[run-probe-layer] image=$IMAGE network=${PROBE_NETWORK:-bridge}" >&2

ENV_ARGS=()
if [[ -n "${PROBE_API_KEY:-}" ]]; then
  ENV_ARGS+=(-e "PROBE_API_KEY=$PROBE_API_KEY")
fi

docker run --rm \
  --network="${PROBE_NETWORK:-bridge}" \
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
