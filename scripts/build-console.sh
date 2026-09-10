#!/usr/bin/env bash
# build-console.sh — reproducible console build in a pinned node container.
# Same pattern as build-hub.sh: toolchain containerized, lockfile-pinned,
# output written back to the repo tree (hub serves dist/ from disk).
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# VITE_CARTO_KEY (optional) is baked into the bundle: set → CARTO dark_all
# primary basemap; unset → Esri primary (see console/src/pages/Radar.tsx).
# Fallback chain so a missing console-build.env never silently degrades the
# basemap: explicit env → core/console-build.env → CARTO_BASEMAP_KEY in
# core/secrets.env.
if [[ -z "${VITE_CARTO_KEY:-}" ]]; then
  if [[ -f "$HUB_DIR/core/console-build.env" ]]; then
    VITE_CARTO_KEY=$(grep '^VITE_CARTO_KEY=' "$HUB_DIR/core/console-build.env" | head -1 | cut -d= -f2- || true)
  fi
  if [[ -z "${VITE_CARTO_KEY:-}" && -f "$HUB_DIR/core/secrets.env" ]]; then
    VITE_CARTO_KEY=$(grep '^CARTO_BASEMAP_KEY=' "$HUB_DIR/core/secrets.env" | head -1 | cut -d= -f2- || true)
  fi
fi
if [[ -z "${VITE_CARTO_KEY:-}" ]]; then
  echo "WARN: no CARTO key found — console will use the Esri fallback basemap" >&2
fi
docker run --rm \
  -e VITE_CARTO_KEY="${VITE_CARTO_KEY:-}" \
  -v "$HUB_DIR/console:/src" \
  -v intelhub-npm-cache:/root/.npm \
  -w /src node:22-trixie \
  sh -c 'npm ci && npm run build'

sudo chown -R zou:zou "$HUB_DIR/console/dist" "$HUB_DIR/console/node_modules" 2>/dev/null || true
echo "==> console built: $HUB_DIR/console/dist"
