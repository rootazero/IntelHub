#!/usr/bin/env bash
# set-dark-map-key.sh — write Stadia (preferred) or CARTO (fallback) basemap
# key into console-build.env and rebuild the console so the dark tiles take
# effect.
#
# The chain order in console/src/pages/Radar.tsx prefers Stadia when both
# are keyed. Stadia's alidade_smooth_dark style is a true-black, OSINT-friendly
# theme (designed as a data-overlay canvas). Get a free key at:
#   https://stadiamaps.com/  (non-commercial use, 200K credits/month)
#
# Without any key the Radar page falls back to Esri's "dark gray" style
# (still functional, less aesthetic). For CARTO-only key writes, see
# set-carto-key.sh.
#
# Usage:
#   bash scripts/set-dark-map-key.sh                     # interactive prompt
#   INTELHUB_NONINTERACTIVE=1 bash scripts/set-dark-map-key.sh  # no prompt (fails)
#   echo <key> | bash scripts/set-dark-map-key.sh -       # pipe key
#   bash scripts/set-dark-map-key.sh <key>               # positional arg
#
# Idempotent: re-running with a new key overwrites the old one.

set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENV="$HUB_DIR/core/console-build.env"

read_key() {
  if [[ $# -gt 0 && "${1:-}" != "-" ]]; then
    echo "$1"; return
  fi
  if [[ "${INTELHUB_NONINTERACTIVE:-0}" == "1" ]]; then
    echo "ERROR: INTELHUB_NONINTERACTIVE=1 but no key supplied" >&2
    exit 1
  fi
  if [[ ! -t 0 ]]; then
    read -r key
    echo "$key"; return
  fi
  local prompt="Stadia Maps API key (https://stadiamaps.com/, free non-commercial; or press Enter for CARTO): "
  read -r -p "$prompt" key
  echo "$key"
}

key="$(read_key "$@")"
if [[ -z "$key" ]]; then
  echo "ERROR: empty key" >&2
  exit 1
fi

# Stadia public domain keys are uuid-shaped (lowercase hex with dashes).
# CARTO free keys start with cb1_. Detect which by shape.
var=""
if [[ "$key" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$ ]]; then
  var="VITE_STADIA_KEY"
  echo "→ detected Stadia key shape"
elif [[ "$key" =~ ^cb1_[a-zA-Z0-9_]+$ ]]; then
  var="VITE_CARTO_KEY"
  echo "→ detected CARTO key shape"
else
  echo "WARN: key doesn't match Stadia (uuid) or CARTO (cb1_...) shapes. Treating as CARTO for backwards compat." >&2
  var="VITE_CARTO_KEY"
fi

mkdir -p "$(dirname "$BENV")"
[[ -f "$BENV" ]] || { echo "# console build-time inputs (0600)" > "$BENV"; }
chmod 600 "$BENV"

if grep -q "^${var}=" "$BENV" 2>/dev/null; then
  sed -i.bak "s|^${var}=.*|${var}=$key|" "$BENV"
  rm -f "$BENV.bak"
else
  echo "${var}=$key" >> "$BENV"
fi
chmod 600 "$BENV"
echo "✓ wrote $BENV"

# Rebuild console to bake the key into the bundle
echo "==> rebuilding console (${var} now baked into bundle)"
bash "$HUB_DIR/scripts/build-console.sh"

echo ""
echo "✓ done. Reload the Radar page in the browser — the dark basemap should"
echo "  appear within ~30s (no hub-core restart needed). The Stadia style is"
echo "  a true black, the CARTO style is also dark grey. Esri fallback is"
echo "  used when no key is set."