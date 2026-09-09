#!/usr/bin/env bash
# deploy.sh — push repo to IntelHub (run from the Mac). Secrets stay VM-side.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rsync -az --delete \
  --exclude '.git/' --exclude 'backups/' --exclude '.DS_Store' \
  --exclude 'compose/.env' --exclude 'docs/' \
  "$REPO/" IntelHub:/home/zou/IntelHub/
ssh IntelHub 'chmod +x /home/zou/IntelHub/scripts/*.sh'
echo "deployed to IntelHub:/home/zou/IntelHub"
