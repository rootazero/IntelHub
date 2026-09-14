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

# 2026-09-14: pre-build verify — if there's an existing dist that was
# produced elsewhere (e.g. rsynced from a Mac dev where the env file
# has keys but the build was done with empty env vars), the new build
# will overwrite it with the correct bundle, so aborting here would
# only lock the operator out. Instead, the check goes AFTER the
# build, comparing the bundle's actual contents to the keys the host
# should have — if the script's resolve_key succeeded but Vite still
# failed to inline (e.g. broken -e injection, a typo'd env var name),
# that's a real bug we want to surface.
#
# Verify scenario: dist exists, host has key K in console-build.env,
# the just-completed build should have inlined K into the bundle.
# If K is NOT in the bundle → abort, don't publish the broken dist.
# (NB: the docker run already wrote dist by the time we get here, so
# on abort the broken dist is left on disk — acceptable because the
# operator will see the ABORT message, restore the env, and re-run.)
DIST_ASSETS="$HUB_DIR/console/dist/assets"
if [[ -d "$DIST_ASSETS" ]]; then
  MISSING=()
  if [[ -n "${VITE_STADIA_KEY:-}" ]] && ! grep -rqF "$VITE_STADIA_KEY" "$DIST_ASSETS" 2>/dev/null; then
    MISSING+=("VITE_STADIA_KEY")
  fi
  if [[ -n "${VITE_CARTO_KEY:-}" ]] && ! grep -rqF "$VITE_CARTO_KEY" "$DIST_ASSETS" 2>/dev/null; then
    MISSING+=("VITE_CARTO_KEY")
  fi
  if (( ${#MISSING[@]} > 0 )); then
    echo "==> ABORT: bundle built WITHOUT ${MISSING[*]} but core/console-build.env has them." >&2
    echo "    Likely cause: build ran on a host without these env vars (e.g. Mac dev)," >&2
    echo "    or the dist was rsynced from elsewhere. Restore core/console-build.env" >&2
    echo "    and re-run build-console.sh on the host that holds the keys." >&2
    exit 1
  fi
fi

# Forensic stamp — operator can spot a Mac-build after the fact even if
# verify passed (e.g. on a host that legitimately has no keys configured,
# the stamp records `stadia:false,carto:false` and the source hostname).
printf '{"ts":"%s","host":"%s","stadia":%s,"carto":%s}\n' \
  "$(date -u +%FT%TZ)" "$(hostname -s 2>/dev/null || echo unknown)" \
  "$([[ -n "${VITE_STADIA_KEY:-}" ]] && echo true || echo false)" \
  "$([[ -n "${VITE_CARTO_KEY:-}" ]] && echo true || echo false)" \
  > "$HUB_DIR/console/dist/.build-manifest.json"

echo "==> console built: $HUB_DIR/console/dist"