#!/usr/bin/env bash
# build-spiderfoot.sh — build SpiderFoot from the pinned upstream commit.
# Upstream publishes no prebuilt image; we build locally for reproducibility
# (manifest: spiderfoot.yml). Idempotent: skips if the image already exists.
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$HUB_DIR/compose/.env"

SPIDERFOOT_COMMIT=$(grep '^SPIDERFOOT_COMMIT=' "$ENV_FILE" | cut -d= -f2)
SPIDERFOOT_IMAGE=$(grep '^SPIDERFOOT_IMAGE=' "$ENV_FILE" | cut -d= -f2)
[[ -z "$SPIDERFOOT_COMMIT" || -z "$SPIDERFOOT_IMAGE" ]] && {
  echo "ERROR: SPIDERFOOT_COMMIT/SPIDERFOOT_IMAGE missing in .env — run resolve-versions.sh first" >&2; exit 1; }

if docker image inspect "$SPIDERFOOT_IMAGE" >/dev/null 2>&1; then
  echo "image already built: $SPIDERFOOT_IMAGE"
  exit 0
fi

BUILD_DIR="$HUB_DIR/build/spiderfoot"
echo "==> fetching smicallef/spiderfoot @ $SPIDERFOOT_COMMIT"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"
git -C "$BUILD_DIR" init -q
git -C "$BUILD_DIR" remote add origin https://github.com/smicallef/spiderfoot.git
git -C "$BUILD_DIR" fetch -q --depth 1 origin "$SPIDERFOOT_COMMIT"
git -C "$BUILD_DIR" checkout -q FETCH_HEAD

echo "==> building $SPIDERFOOT_IMAGE"
docker build -q -t "$SPIDERFOOT_IMAGE" "$BUILD_DIR"
echo "==> built: $SPIDERFOOT_IMAGE"
