#!/usr/bin/env bash
# hub-compose.sh — single entry point for the multi-file compose stack.
# Usage: scripts/hub-compose.sh [compose args...]   e.g.  scripts/hub-compose.sh ps
set -euo pipefail
COMPOSE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../compose" && pwd)"
cd "$COMPOSE_DIR"
exec docker compose --env-file .env \
  -f compose.base.yml \
  -f compose.data.yml \
  -f compose.sensor.yml \
  -f compose.ui.yml \
  "$@"
