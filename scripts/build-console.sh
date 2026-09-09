#!/usr/bin/env bash
# build-console.sh — reproducible console build in a pinned node container.
# Same pattern as build-hub.sh: toolchain containerized, lockfile-pinned,
# output written back to the repo tree (hub serves dist/ from disk).
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

docker run --rm \
  -v "$HUB_DIR/console:/src" \
  -v intelhub-npm-cache:/root/.npm \
  -w /src node:22-trixie \
  sh -c 'npm ci && npm run build'

sudo chown -R zou:zou "$HUB_DIR/console/dist" "$HUB_DIR/console/node_modules" 2>/dev/null || true
echo "==> console built: $HUB_DIR/console/dist"
