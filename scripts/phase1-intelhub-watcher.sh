#!/usr/bin/env bash
# Background watcher for IntelHub production recovery.
# Polls 10.10.10.41 (IntelHub VM) and 10.10.10.40 (PVE40 host)
# every 60s. When both come back, runs the Phase 1 acceptance
# checks automatically. Times out after 6h.
#
# Designed to be launched in background by the agent; writes
# status to /tmp/phase1-intelhub-watcher.log for visibility.
set +e
TIMEOUT=21600   # 6h
START=$(date +%s)
LOG=/tmp/phase1-intelhub-watcher.log
: > "$LOG"
while true; do
  NOW=$(date +%s)
  ELAPSED=$((NOW - START))
  if [ "$ELAPSED" -gt "$TIMEOUT" ]; then
    echo "[$(date -u +%H:%M:%S)] TIMEOUT after ${ELAPSED}s, exiting" >> "$LOG"
    exit 0
  fi
  # Check IntelHub
  INTEL=$(ssh -o BatchMode=yes -o ConnectTimeout=5 IntelHub 'echo alive' 2>/dev/null | head -1)
  PVE=$(timeout 3 bash -c "</dev/tcp/10.10.10.40/22" 2>/dev/null && echo OK || echo FAIL)
  if [ "$INTEL" = "alive" ] && [ "$PVE" = "OK" ]; then
    echo "[$(date -u +%H:%M:%S)] INTELHUB+PVE40 BACK — running Phase 1 sp6" >> "$LOG"
    ssh -o BatchMode=yes IntelHub bash <<'EOF' >> "$LOG" 2>&1
cd /home/zou/IntelHub
KEY=$(grep -o "ihk_[a-f0-9]*" core/agent-keys.txt | head -1)
echo "--- hub-core status:"
systemctl is-active hub-core
echo "--- Phase 1 sp6 results:"
python3 scripts/accept-sp6.py "$KEY" 2>&1 \
  | grep -E "open_meteo|usgs_water|open_aq|hdx_humanitarian|nager_date|sunrise_sunset|queimadas_inpe|helium_news|==.*(passed|failed|shelved)"
EOF
    echo "[$(date -u +%H:%M:%S)] ACCEPTANCE DONE — see above" >> "$LOG"
    exit 0
  fi
  echo "[$(date -u +%H:%M:%S)] intel=${INTEL:-timeout} pve40=${PVE} elapsed=${ELAPSED}s" >> "$LOG"
  sleep 60
done