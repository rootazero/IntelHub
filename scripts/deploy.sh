#!/usr/bin/env bash
# deploy.sh — push repo to IntelHub (run from the Mac). Secrets stay VM-side.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# NOTE: config/searxng/ is excluded — the container chowns it at runtime;
# settings changes are applied VM-side with sudo.
#
# The `console/dist/` exclude is load-bearing: dist is build output that
# Vite produces on the VM at deploy time (build-console.sh), baking in
# keys from core/console-build.env via docker -e. A Mac-built dist has
# empty keys (no VITE_STADIA_KEY / VITE_CARTO_KEY in Mac env) and
# causes the "DARK MAP KEY MISSING" badge on the radar page. Never
# rsync console/dist/ from the Mac — always let the VM build it.
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  --exclude 'console/node_modules/' --exclude 'console/dist/' \
  --exclude 'core/' --exclude 'data/' \
  "$REPO/" IntelHub:/home/zou/IntelHub/
ssh IntelHub 'chmod +x /home/zou/IntelHub/scripts/*.sh'
echo "deployed to IntelHub:/home/zou/IntelHub"
echo "→ run 'bash scripts/build-console.sh' on the VM to bake the dark-map keys into console/dist/"
echo "  (or run 'bash scripts/update.sh' which does this automatically)"
