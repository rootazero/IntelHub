#!/usr/bin/env bash
# rollback.sh <component> — restore last_known_good pin and redeploy (spec §5).
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$HUB_DIR"
COMPOSE="scripts/hub-compose.sh"

COMPONENT="${1:?usage: rollback.sh <component>}"
case "$COMPONENT" in
  postgres) VAR=POSTGRES_VERSION ;;  redis) VAR=REDIS_VERSION ;;
  neo4j)    VAR=NEO4J_VERSION ;;     qdrant) VAR=QDRANT_VERSION ;;
  searxng)  VAR=SEARXNG_VERSION ;;   crawl4ai) VAR=CRAWL4AI_VERSION ;;
  *) echo "unknown component: $COMPONENT" >&2; exit 1 ;;
esac

CUR=$(grep "^${VAR}=" compose/.env | cut -d= -f2)
PREV=$(grep "^${VAR}_PREVIOUS=" compose/.env | cut -d= -f2 || true)
[[ -z "$PREV" ]] && { echo "no last_known_good recorded for $COMPONENT" >&2; exit 1; }

echo "==> rollback $COMPONENT: $CUR → $PREV"
sed -i "s/^${VAR}=.*/${VAR}=${PREV}/" compose/.env
sed -i "s/^${VAR}_PREVIOUS=.*/${VAR}_PREVIOUS=${CUR}/" compose/.env
$COMPOSE up -d "$COMPONENT"

for i in $(seq 1 24); do
  H=$(docker inspect -f '{{.State.Health.Status}}' "intelhub-${COMPONENT}" 2>/dev/null || echo missing)
  [[ "$H" == "healthy" ]] && break
  sleep 5
done
[[ "$H" == "healthy" ]] || { echo "ERROR: rollback target not healthy — restore from backups/" >&2; exit 1; }
echo "==> rollback complete: $COMPONENT healthy on $PREV"
