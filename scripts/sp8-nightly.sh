#!/usr/bin/env bash
# sp8-nightly.sh — VM-native nightly acceptance run.
#
# Runs the full sp8 acceptance suite (including motion + multi-layer probes
# via SP8_MOTION_PROBE=1) at 03:00 UTC nightly. Writes a structured summary
# to a Redis hash so sp8 (and the dashboard) can read freshness + last-run
# status without scraping logs.
#
# The wrapper invokes `python3 scripts/accept-sp8.py` directly on the hub VM
# because the probe infrastructure (docker, node:22-trixie, chromium cache)
# all lives here — ssh-ing from Mac every night would re-pay the chromium
# download cost on every run. The Mac side reads the Redis heartbeat via
# the same `_remote.redis` helper sp8 uses.
#
# Redis schema:
#   hub:nightly:sp8:run  HSET  ts=ISO8601  host=IntelHub  passed=N  shelved=N
#                                  deferred=N  failed=N  log=path  status=ok|fail
#
# On success: status=ok, exit 0. On failure: status=fail, exit 1. Cron
# failures propagate to systemd as a failed service unit.
#
# Usage (manual):
#   bash /home/zou/IntelHub/scripts/sp8-nightly.sh [base-url]
#
# Invocation from systemd sp8-nightly.service (which sets cwd + env).
set -uo pipefail

BASE="${1:-http://10.10.10.41:8800}"
KEY=$(grep -o "ihk_[a-f0-9]*" /home/zou/IntelHub/core/agent-keys.txt 2>/dev/null | head -1)
# User-writable XDG log dir — /var/log/intelhub requires sudo on a fresh
# install and would force sudo on every systemd run. ~/.local/share is
# always writable by the user running the service (User=zou).
LOG_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/intelhub/logs"
LOG_FILE="$LOG_DIR/sp8-nightly-$(date -u +%Y-%m-%d).log"
mkdir -p "$LOG_DIR"

cd /home/zou/IntelHub

export SP8_MOTION_PROBE=1

# Stamp a start marker so concurrent runs (manual + scheduled) are visible.
START_TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
echo "[sp8-nightly] start=$START_TS base=$BASE" | tee -a "$LOG_FILE"

# Capture stdout/stderr to log + transient file for parsing the summary line.
RAW_OUT=$(mktemp)
set +e
python3 scripts/accept-sp8.py "$KEY" "$BASE" 2>&1 | tee -a "$LOG_FILE" > "$RAW_OUT"
RC=${PIPESTATUS[0]}
set -e

# Parse the trailing summary line: "== N passed, M shelved, K deferred, J failed =="
SUMMARY=$(grep -E '^== [0-9]+ passed, [0-9]+ shelved, [0-9]+ deferred, [0-9]+ failed ==' "$RAW_OUT" | tail -1)
PASSED=$(echo "$SUMMARY" | sed -E 's/.*== ([0-9]+) passed.*/\1/')
SHELVED=$(echo "$SUMMARY" | sed -E 's/.*, ([0-9]+) shelved.*/\1/')
DEFERRED=$(echo "$SUMMARY" | sed -E 's/.*, ([0-9]+) deferred.*/\1/')
FAILED=$(echo "$SUMMARY" | sed -E 's/.*, ([0-9]+) failed.*/\1/')
END_TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)

STATUS="ok"
if [[ "$RC" -ne 0 ]] || [[ "${FAILED:-0}" -ne 0 ]]; then
  STATUS="fail"
fi

# Write Redis heartbeat. Password extracted from compose/.env (same pattern
# as scripts/_remote.py uses for its redis() helper).
REDIS_PASS=$(grep '^REDIS_PASSWORD=' /home/zou/IntelHub/compose/.env 2>/dev/null | cut -d= -f2-)
if [[ -n "$REDIS_PASS" ]]; then
  docker exec -e REDIS_PASSWORD="$REDIS_PASS" intelhub-redis \
    redis-cli --no-auth-warning -a "$REDIS_PASS" HSET hub:nightly:sp8:run \
      ts "$END_TS" \
      host "$(hostname)" \
      passed "${PASSED:-?}" \
      shelved "${SHELVED:-?}" \
      deferred "${DEFERRED:-?}" \
      failed "${FAILED:-?}" \
      status "$STATUS" \
      log "$LOG_FILE" \
    >/dev/null
fi

rm -f "$RAW_OUT"
echo "[sp8-nightly] end=$END_TS summary=\"$SUMMARY\" status=$STATUS" | tee -a "$LOG_FILE"
exit $RC
