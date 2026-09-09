#!/usr/bin/env bash
# deploy.sh — push repo to IntelHub (run from the Mac). Secrets stay VM-side.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# NOTE: config/searxng/ is excluded — the container chowns it at runtime;
# settings changes are applied VM-side with sudo.
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'docs/' --exclude 'build/' \
  --exclude 'config/searxng/' --exclude 'hub-core/target/' \
  "$REPO/" IntelHub:/home/zou/IntelHub/
ssh IntelHub 'chmod +x /home/zou/IntelHub/scripts/*.sh'
echo "deployed to IntelHub:/home/zou/IntelHub"
