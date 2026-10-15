#!/usr/bin/env bash
# Record what Twenty's integration tests do to Redis and PostgreSQL.
# Usage: lab/twenty/record.sh <twenty checkout> [jest test path pattern]
#
# Redis: every command via MONITOR (the prototype cannot parse MGET/MSET).
# PostgreSQL: every committed row change via a test_decoding slot created
# after the database reset, read once the tests finish.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TWENTY=$(cd "${1:?twenty checkout}" && pwd)
PATTERN=${2:-}
OUT="$ROOT/lab/out/twenty-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
node_run() {
  docker run --rm --network host -u "$(id -u):$(id -g)" -e HOME=/tmp/h -e NODE_ENV=test \
    -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm bash -c \
    "mkdir -p /tmp/h/bin && corepack enable --install-directory /tmp/h/bin && export PATH=/tmp/h/bin:\$PATH && $1"
}

echo "reset database (build + drop schemas + migrate)"
node_run "NODE_OPTIONS='--max-old-space-size=8192' npx nx database:reset" > "$OUT/reset.log" 2>&1

docker exec cw-twenty-pg-1 psql -qU postgres -d test \
  -c "select pg_drop_replication_slot('cw') from pg_replication_slots where slot_name='cw'" \
  -c "select 1 from pg_create_logical_replication_slot('cw','test_decoding')" >/dev/null
docker exec cw-twenty-redis-1 redis-cli flushall >/dev/null
docker exec cw-twenty-redis-1 redis-cli monitor > "$OUT/monitor.log" &
MON=$!
trap 'kill $MON 2>/dev/null || true' EXIT

echo "run integration tests ${PATTERN:-(all)}"
# printf %q keeps a regex like "a|b" from becoming a shell pipe inside bash -c.
node_run "NODE_OPTIONS='--max-old-space-size=8192' npx jest --config ./jest-integration.config.ts --runInBand $(printf %q "$PATTERN")" \
  > "$OUT/jest.log" 2>&1 || echo "jest exited non-zero (see jest.log)"

sleep 1
kill $MON 2>/dev/null || true
docker exec cw-twenty-pg-1 psql -qtAU postgres -d test \
  -c "select data from pg_logical_slot_get_changes('cw', null, null, 'include-timestamp', 'on')" > "$OUT/wal.log"
echo "out: $OUT"
wc -l "$OUT"/monitor.log "$OUT"/wal.log
