#!/usr/bin/env bash
# upgrade.sh <component> <new-version> — mandated upgrade flow (spec §5):
#   backup → pull → disposable test instance → health probe → promote → verify
# Never a bare `compose pull && up -d`. On any failure: rollback.sh.
set -euo pipefail
HUB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$HUB_DIR"
COMPOSE="scripts/hub-compose.sh"

COMPONENT="${1:?usage: upgrade.sh <component> <new-version>}"
NEW="${2:?usage: upgrade.sh <component> <new-version>}"

# component → (env var, image repo, probe command for disposable instance)
case "$COMPONENT" in
  postgres)  VAR=POSTGRES_VERSION; REPO="postgres";            PROBE="pg_isready" ;;
  redis)     VAR=REDIS_VERSION;    REPO="redis";               PROBE="redis-server --version" ;;
  neo4j)     VAR=NEO4J_VERSION;    REPO="neo4j";               PROBE="cypher-shell --version" ;;
  qdrant)    VAR=QDRANT_VERSION;   REPO="qdrant/qdrant";       PROBE="./qdrant --version" ;;
  searxng)   VAR=SEARXNG_VERSION;  REPO="searxng/searxng";     PROBE="python3 --version" ;;
  crawl4ai)  VAR=CRAWL4AI_VERSION; REPO="unclecode/crawl4ai";  PROBE="python3 --version" ;;
  *) echo "unknown component: $COMPONENT" >&2; exit 1 ;;
esac

OLD=$(grep "^${VAR}=" compose/.env | cut -d= -f2)
[[ "$OLD" == "$NEW" ]] && { echo "already at $NEW"; exit 0; }
IMAGE="${REPO}:${NEW}"

echo "==> upgrade $COMPONENT: $OLD → $NEW"
echo "==> [1/5] backup"
scripts/backup.sh

echo "==> [2/5] pull $IMAGE"
docker pull "$IMAGE"

echo "==> [3/5] disposable test instance"
TEST="${COMPONENT}-upgtest-$$"
docker rm -f "$TEST" >/dev/null 2>&1 || true
if ! docker run -d --rm --name "$TEST" "$IMAGE" >/dev/null 2>&1; then
  echo "ERROR: test instance failed to start; aborting (pin unchanged)" >&2; exit 1
fi
sleep 8
if ! docker exec "$TEST" sh -c "$PROBE" >/dev/null 2>&1; then
  echo "ERROR: test instance probe failed; aborting (pin unchanged)" >&2
  docker rm -f "$TEST" >/dev/null 2>&1 || true
  exit 1
fi
docker rm -f "$TEST" >/dev/null 2>&1 || true
echo "    test instance OK"

echo "==> [4/5] promote: $VAR=$NEW (last_known_good=$OLD)"
sed -i "s/^${VAR}=.*/${VAR}=${NEW}/" compose/.env
grep -q "^${VAR}_PREVIOUS=" compose/.env \
  && sed -i "s/^${VAR}_PREVIOUS=.*/${VAR}_PREVIOUS=${OLD}/" compose/.env \
  || echo "${VAR}_PREVIOUS=${OLD}" >> compose/.env
$COMPOSE up -d "$COMPONENT"

echo "==> [5/5] post-upgrade health"
for i in $(seq 1 24); do
  H=$(docker inspect -f '{{.State.Health.Status}}' "intelhub-${COMPONENT}" 2>/dev/null || echo missing)
  [[ "$H" == "healthy" ]] && break
  sleep 5
done
if [[ "$H" != "healthy" ]]; then
  echo "ERROR: $COMPONENT not healthy after upgrade — rolling back" >&2
  scripts/rollback.sh "$COMPONENT"
  exit 1
fi
echo "==> upgrade complete: $COMPONENT is healthy on $NEW"
