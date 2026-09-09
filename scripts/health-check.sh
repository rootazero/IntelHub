#!/usr/bin/env bash
# health-check.sh — one-page status of the whole hub (spec §5).
set -uo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "=== IntelHub health $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
echo
echo "-- host --"
uptime
free -h | awk 'NR==1||/Mem/'
df -h / | awk 'NR==1||NR==2'
echo
echo "-- containers --"
docker ps -a --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}' | sed 's/\(.\{100\}\).*/\1/'
echo
echo "-- resources (snapshot) --"
docker stats --no-stream --format 'table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}'
echo
echo "-- compose config check --"
"$HUB_DIR/scripts/hub-compose.sh" config --quiet && echo "compose files: OK"
