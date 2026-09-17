#!/usr/bin/env bash
# Sync vendored GEV engine from upstream pinned SHA. Idempotent.
# Usage: bash scripts/sync-gev-engine.sh [--check]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDOR="$ROOT/console/gev-engine"
META="$VENDOR/UPSTREAM.json"
SHA=$(python3 -c "import json;print(json.load(open('$META'))['pinned_sha'])")
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
git clone --quiet --filter=blob:none https://github.com/bilawalsidhu/gods-eye-view "$TMP/gev"
git -C "$TMP/gev" checkout --quiet "$SHA"
if [[ "${1:-}" == "--check" ]]; then
  REMOTE=$(git -C "$TMP/gev" ls-remote origin HEAD | cut -f1)
  [[ "$REMOTE" == "$SHA" ]] && echo "gev-engine: up to date ($SHA)" || echo "gev-engine: upstream moved HEAD=$REMOTE pinned=$SHA"
  exit 0
fi
rsync -a --delete \
  --exclude '.git' --exclude 'server' --exclude 'src/standalone' \
  --exclude 'src/ui/templates' --exclude 'src/data/local_data' \
  --exclude 'docs' --exclude 'build' --exclude 'index.html' \
  --exclude 'style.css' --exclude 'public' --exclude 'UPSTREAM.json' \
  "$TMP/gev/" "$VENDOR/"
echo "synced gev-engine @ $SHA — now run: cd console && npx vitest run src/gev-boot/__tests__/source-contracts.test.ts"
