#!/usr/bin/env bash
# build-console.sh — reproducible console build in a pinned node container.
# Same pattern as build-hub.sh: toolchain containerized, lockfile-pinned,
# output written back to the repo tree (hub serves dist/ from disk).
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Two optional dark basemap keys can be baked into the bundle:
#   VITE_STADIA_KEY → Stadia Maps alidade_smooth_dark (primary choice)
#   VITE_CARTO_KEY  → CARTO dark_all (legacy primary)
# Without either key the chain starts at Esri (grey, not black).
#
# Fallback chain per key so a missing console-build.env never silently
# degrades the basemap: explicit env → core/console-build.env →
# matching CARTO_BASEMAP_KEY / STADIA_API_KEY in core/secrets.env.
# Order of precedence in the rendered chain: Stadia > CARTO > Esri.

resolve_key() {
  local vite_var="$1" secrets_var="$2"
  local env_var="${!vite_var:-}"
  if [[ -n "$env_var" ]]; then
    echo "$env_var"; return
  fi
  if [[ -f "$HUB_DIR/core/console-build.env" ]]; then
    local v
    v=$(grep "^${vite_var}=" "$HUB_DIR/core/console-build.env" | head -1 | cut -d= -f2- || true)
    if [[ -n "$v" ]]; then echo "$v"; return; fi
  fi
  if [[ -f "$HUB_DIR/core/secrets.env" ]]; then
    local v
    v=$(grep "^${secrets_var}=" "$HUB_DIR/core/secrets.env" | head -1 | cut -d= -f2- || true)
    if [[ -n "$v" ]]; then echo "$v"; return; fi
  fi
  echo ""
}

VITE_STADIA_KEY="$(resolve_key VITE_STADIA_KEY STADIA_API_KEY)"
VITE_CARTO_KEY="$(resolve_key VITE_CARTO_KEY CARTO_BASEMAP_KEY)"

if [[ -z "${VITE_STADIA_KEY:-}" ]] && [[ -z "${VITE_CARTO_KEY:-}" ]]; then
  echo "WARN: no dark basemap key found (Stadia or CARTO) — console will use the Esri fallback (grey, not black). Sign up free at stadiamaps.com or carto.com/basemaps/apikey." >&2
fi
docker run --rm \
  -e VITE_STADIA_KEY="${VITE_STADIA_KEY:-}" \
  -e VITE_CARTO_KEY="${VITE_CARTO_KEY:-}" \
  -v "$HUB_DIR/console:/src" \
  -v intelhub-npm-cache:/root/.npm \
  -w /src node:22-trixie \
  sh -c 'npm ci && npm run build'

sudo chown -R zou:zou "$HUB_DIR/console/dist" "$HUB_DIR/console/node_modules" 2>/dev/null || true
echo "==> console built: $HUB_DIR/console/dist"