#!/usr/bin/env bash
# Race experiment on Twenty: reset the seeded test database, start SERVERS
# Twenty server processes, record Redis MONITOR and a test_decoding slot while
# race.js renames the workspace under concurrent authenticated reads, then
# compare the cached workspace with the database. Everything runs inside
# the isolated lab namespace (see compose.yaml); no traffic leaves the host.
# Usage: [NSERVERS=n] [SKIP_RESET=1] lab/twenty/race.sh <twenty checkout> [KEY=VALUE env for race.js...]
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TWENTY=$(cd "${1:?twenty checkout}" && pwd); shift
NSERVERS=${NSERVERS:-1}
OUT="$ROOT/lab/out/race-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
WS=20202020-1c25-4d02-bf25-6aeccf7ea419   # seeded Apple workspace
TOKEN=$(python3 -I -c 'import json,sys;print(json.load(open(sys.argv[1]))["APPLE_JANE_ADMIN_ACCESS_TOKEN"])' \
  "$TWENTY/packages/twenty-server/test/integration/constants/test-tokens.json")
source "$ROOT/lab/twenty/isolation.sh"
assert_isolated

if [ -z "${SKIP_RESET:-}" ]; then
  echo "reset database"
  node_isolated -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm bash -c \
    "NODE_OPTIONS='--max-old-space-size=8192' npx nx database:reset" > "$OUT/reset.log" 2>&1
fi

SERVERS=""
for i in $(seq 0 $((NSERVERS - 1))); do
  port=$((3000 + i))
  node_isolated -d --name "cw-twenty-server-$i" -e NODE_PORT=$port \
    -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm node dist/main > /dev/null
  SERVERS="${SERVERS:+$SERVERS,}http://127.0.0.1:$port"
done
cleanup() {
  for i in $(seq 0 $((NSERVERS - 1))); do
    docker logs "cw-twenty-server-$i" > "$OUT/server-$i.log" 2>&1 || true
    docker rm -f "cw-twenty-server-$i" > /dev/null 2>&1 || true
  done
  kill "${MON:-0}" 2>/dev/null || true
}
trap cleanup EXIT
for i in $(seq 0 $((NSERVERS - 1))); do wait_port $((3000 + i)); done
echo "servers up: $SERVERS"

docker exec cw-twenty-pg-1 psql -qU postgres -d test \
  -c "select pg_drop_replication_slot('cw') from pg_replication_slots where slot_name='cw'" \
  -c "select 1 from pg_create_logical_replication_slot('cw','test_decoding')" > /dev/null
docker exec cw-twenty-redis-1 redis-cli monitor > "$OUT/monitor.log" &
MON=$!

node_isolated -v "$ROOT/lab/twenty":/h:ro -v "$OUT":/out -w /out \
  -e SERVERS="$SERVERS" -e TOKEN="$TOKEN" -e OUT=/out "${@/#/-e}" node:24.16.0-bookworm node /h/race.js | tee "$OUT/race.out"

sleep 2
kill $MON 2>/dev/null || true
# Final state: what the cache holds versus what the database committed.
docker exec cw-twenty-redis-1 redis-cli get "integration-tests:engine:core-entity:workspace:$WS:data" > "$OUT/final-cache.json"
docker exec cw-twenty-pg-1 psql -qtAU postgres -d test -c "select \"displayName\" from core.workspace where id='$WS'" > "$OUT/final-db.txt"
docker exec cw-twenty-pg-1 psql -qtAU postgres -d test \
  -c "select data from pg_logical_slot_get_changes('cw', null, null, 'include-timestamp', 'on')" > "$OUT/wal.log"
echo "out: $OUT"
echo "final db: $(cat "$OUT/final-db.txt")  final cache: $(python3 -I -c 'import json,sys;d=open(sys.argv[1]).read().strip();print(json.loads(d).get("displayName") if d else None)' "$OUT/final-cache.json")"
