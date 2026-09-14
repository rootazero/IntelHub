#!/usr/bin/env bash
# set-carto-key.sh — write CARTO basemap key into console-build.env and
# rebuild the console so the dark tiles take effect.
#
# Without a key the Radar page falls back to Esri's "dark gray" style
# (still functional, less aesthetic). Get a free key at:
#   https://carto.com/basemaps/apikey
#
# Usage:
#   bash scripts/set-carto-key.sh                     # interactive prompt
#   INTELHUB_NONINTERACTIVE=1 bash scripts/set-carto-key.sh  # no prompt (fails)
#   echo cb1_xxx | bash scripts/set-carto-key.sh -    # pipe key
#   bash scripts/set-carto-key.sh cb1_xxx             # positional arg
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
    # piped
    read -r key
    echo "$key"; return
  fi
  # interactive
  local prompt="CARTO basemap key (https://carto.com/basemaps/apikey free signup): "
  read -r -p "$prompt" key
  echo "$key"
}

key="$(read_key "$@")"
if [[ -z "$key" ]]; then
  echo "ERROR: empty key" >&2
  exit 1
fi

# cb1_ prefix is CARTO's standard free-tier key shape
if [[ ! "$key" =~ ^cb1_[a-zA-Z0-9_]+$ ]] && [[ "${INTELHUB_NONINTERACTIVE:-0}" != "1" ]]; then
  echo "WARN: key doesn't look like CARTO's cb1_xxx format. Continuing anyway." >&2
fi

mkdir -p "$(dirname "$BENV")"
[[ -f "$BENV" ]] || { echo "# console build-time inputs (0600)" > "$BENV"; }
chmod 600 "$BENV"

# Upsert VITE_CARTO_KEY (no quoting — CARTO keys are alphanumeric+underscore)
if grep -q '^VITE_CARTO_KEY=' "$BENV" 2>/dev/null; then
  # Replace existing line in-place
  sed -i.bak "s|^VITE_CARTO_KEY=.*|VITE_CARTO_KEY=$key|" "$BENV"
  rm -f "$BENV.bak"
else
  echo "VITE_CARTO_KEY=$key" >> "$BENV"
fi
chmod 600 "$BENV"
echo "✓ wrote $BENV"

# Rebuild console to bake the key into the bundle
echo "==> rebuilding console (VITE_CARTO_KEY now baked into bundle)"
bash "$HUB_DIR/scripts/build-console.sh"

echo ""
echo "✓ done. Reload the Radar page in the browser — the dark CARTO basemap"
echo "  should appear within ~30s (no hub-core restart needed)."
