#!/usr/bin/env bash
# build-crucix.sh <git-sha> — build the Crucix image from a pinned upstream
# commit (upstream calesthio/Crucix has no release tags; directive requires
# version pinning, so we pin the commit SHA and tag the image with it).
#
# With CRUCIX_LLM_PATCH=1 (default) the hardcoded OpenAI base URL in
# lib/llm/openai.mjs is rewritten to the T8star relay — deterministic per
# SHA, re-applied on every build (spec SP4 §1 Q4b).
set -euo pipefail

SHA="${1:-}"
if [[ -z "$SHA" ]]; then
  echo "usage: $0 <git-sha>   # e.g. $0 3db7068" >&2
  exit 1
fi

BASE="/home/zou/IntelHub"
BUILD_DIR="$BASE/build/crucix"
TAG="crucix:sha-${SHA:0:7}"
LLM_BASE="${CRUCIX_LLM_BASE:-https://ai.t8star.org/v1}"
PATCH="${CRUCIX_LLM_PATCH:-1}"

mkdir -p "$BUILD_DIR"
if [[ ! -d "$BUILD_DIR/.git" ]]; then
  git clone --quiet https://github.com/calesthio/Crucix.git "$BUILD_DIR"
fi
git -C "$BUILD_DIR" fetch --quiet origin
git -C "$BUILD_DIR" checkout --quiet "$SHA"
git -C "$BUILD_DIR" clean -fdxq

FULL_SHA="$(git -C "$BUILD_DIR" rev-parse HEAD)"
echo "== building crucix @ $FULL_SHA"

if [[ "$PATCH" == "1" ]]; then
  sed -i "s#https://api.openai.com/v1/chat/completions#${LLM_BASE}/chat/completions#" \
    "$BUILD_DIR/lib/llm/openai.mjs"
  grep -q "${LLM_BASE}" "$BUILD_DIR/lib/llm/openai.mjs" \
    && echo "== LLM base-URL patch applied → $LLM_BASE" \
    || { echo "!! LLM patch failed to apply"; exit 1; }
fi

docker build --quiet -t "$TAG" "$BUILD_DIR" >/dev/null
echo "== built $TAG"
echo "   set CRUCIX_IMAGE=$TAG in compose/.env, then: scripts/hub-compose.sh up -d crucix"
