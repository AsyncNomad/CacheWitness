#!/usr/bin/env bash
# Race experiment on Twenty: reset the seeded test database, start SERVERS
# Twenty server processes, record Redis MONITOR and a test_decoding slot while
# race.js renames the workspace under concurrent authenticated reads, then
# compare the cached workspace with the database. Everything runs inside
# the isolated lab namespace (see compose.yaml); no traffic leaves the host.
# Usage: [NSERVERS=n] [SKIP_RESET=1] [PG_DELAY_MS=d] [AMP_DELAY_MS=d] \
#        lab/twenty/race.sh <twenty checkout> [KEY=VALUE env for race.js...]
#   PG_DELAY_MS   netem delay on every packet to or from PostgreSQL (port 5432)
#                 inside the lab namespace, so each round trip gains 2*d ms
#   AMP_DELAY_MS  route the servers' Redis traffic through amp-proxy.js, which
#                 holds SET/MSET of the workspace entity for d ms
#   REFILL_DELAY_MS  sleep d ms between the DB read and the cache write in
#                 Twenty's refill path (needs refill-delay.patch applied and built)
#   JITTER=1      either delay becomes a uniform random 0..d ms per write
# Manipulation check: when a delay is requested, the run fails unless the
# delay was applied to exactly the refill writes Redis executed.
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
# Leftovers from an interrupted run must not leak into this one.
docker rm -f cw-twenty-amp > /dev/null 2>&1 || true
docker run --rm --network container:cw-twenty-pg-1 --cap-add NET_ADMIN cw-netem:local tc qdisc del dev lo root 2>/dev/null || true

if [ -z "${SKIP_RESET:-}" ]; then
  echo "reset database"
  node_isolated -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm bash -c \
    "NODE_OPTIONS='--max-old-space-size=8192' npx nx database:reset" > "$OUT/reset.log" 2>&1
fi

SERVER_ENV=()
if [ -n "${AMP_DELAY_MS:-}" ]; then
  CW_KEEP=1 node_isolated -d --name cw-twenty-amp -v "$ROOT/lab/twenty":/h:ro -e AMP_DELAY_MS="$AMP_DELAY_MS" \
    -e AMP_JITTER="${JITTER:-0}" \
    node:24.16.0-bookworm node /h/amp-proxy.js > /dev/null
  wait_port 6380
  # Twenty's .env.test wins over process environment variables, so the
  # servers get their own copy of it pointing at the proxy.
  sed 's|^REDIS_URL=.*|REDIS_URL=redis://127.0.0.1:6380|' "$TWENTY/packages/twenty-server/.env.test" > "$OUT/env.test"
  SERVER_ENV+=(-v "$OUT/env.test":/w/packages/twenty-server/.env.test:ro)
  echo "amplifier: SET/MSET of the workspace entity held ${AMP_DELAY_MS}ms"
fi
if [ -n "${REFILL_DELAY_MS:-}" ]; then
  SERVER_ENV+=(-e CW_REFILL_DELAY_MS="$REFILL_DELAY_MS" -e CW_REFILL_JITTER="${JITTER:-0}")
  echo "refill delay in Twenty's code: ${REFILL_DELAY_MS}ms"
fi
if [ -n "${PG_DELAY_MS:-}" ]; then
  # prio qdisc with every band mapped to 1:2, and port-5432 traffic sent to 1:3 (netem).
  docker run --rm --network container:cw-twenty-pg-1 --cap-add NET_ADMIN cw-netem:local sh -c "
    tc qdisc add dev lo root handle 1: prio bands 3 priomap 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 &&
    tc qdisc add dev lo parent 1:3 handle 30: netem delay ${PG_DELAY_MS}ms &&
    tc filter add dev lo parent 1: protocol ip prio 1 u32 match ip sport 5432 0xffff flowid 1:3 &&
    tc filter add dev lo parent 1: protocol ip prio 2 u32 match ip dport 5432 0xffff flowid 1:3"
  echo "netem: +${PG_DELAY_MS}ms each way on PostgreSQL traffic"
fi

SERVERS=""
for i in $(seq 0 $((NSERVERS - 1))); do
  port=$((3000 + i))
  node_isolated -d --name "cw-twenty-server-$i" -e NODE_PORT=$port "${SERVER_ENV[@]}" \
    -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm node dist/main > /dev/null
  SERVERS="${SERVERS:+$SERVERS,}http://127.0.0.1:$port"
done
cleanup() {
  for i in $(seq 0 $((NSERVERS - 1))); do
    docker logs "cw-twenty-server-$i" > "$OUT/server-$i.log" 2>&1 || true
    docker rm -f "cw-twenty-server-$i" > /dev/null 2>&1 || true
  done
  kill "${MON:-0}" 2>/dev/null || true
  if [ -n "${AMP_DELAY_MS:-}" ]; then
    docker logs cw-twenty-amp > "$OUT/amp.log" 2>&1 || true
    docker rm -f cw-twenty-amp > /dev/null 2>&1 || true
  fi
  if [ -n "${PG_DELAY_MS:-}" ]; then
    docker run --rm --network container:cw-twenty-pg-1 --cap-add NET_ADMIN cw-netem:local tc qdisc del dev lo root || true
  fi
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

sleep 6   # lets the delay counters log once more
kill $MON 2>/dev/null || true

# Manipulation check: the requested delay must have hit every refill write.
refill_sets=$(grep -cE '"(SET|MSET)" .*engine:core-entity:workspace:[^:"]+:data"' "$OUT/monitor.log" || true)
check() { echo "$1" > "$OUT/manipulation.txt"; echo "$1"; }
if [ -n "${AMP_DELAY_MS:-}" ]; then
  docker stop -t 3 cw-twenty-amp > /dev/null 2>&1 || true
  held=$( (docker logs cw-twenty-amp 2>&1 || true) | sed -n 's/.*"held":\([0-9]*\).*/\1/p' | tail -1)
  if [ "${held:-0}" -gt 0 ] && [ "$held" -eq "$refill_sets" ]; then
    check "OK amplifier held=$held refill_sets=$refill_sets"
  else
    check "INVALID amplifier held=${held:-0} refill_sets=$refill_sets"; exit 3
  fi
fi
if [ -n "${REFILL_DELAY_MS:-}" ]; then
  applied=0
  for i in $(seq 0 $((NSERVERS - 1))); do
    a=$(docker logs "cw-twenty-server-$i" 2>&1 | sed -n 's/.*cw-refill-delay ms=[0-9]* applied=\([0-9]*\).*/\1/p' | tail -1)
    applied=$((applied + ${a:-0}))
  done
  if [ "$applied" -gt 0 ] && [ "$applied" -eq "$refill_sets" ]; then
    check "OK refill-delay applied=$applied refill_sets=$refill_sets"
  else
    check "INVALID refill-delay applied=$applied refill_sets=$refill_sets"; exit 3
  fi
fi
if [ -z "${AMP_DELAY_MS:-}${REFILL_DELAY_MS:-}" ]; then
  check "OK baseline refill_sets=$refill_sets"
fi
# Final state: what the cache holds versus what the database committed.
docker exec cw-twenty-redis-1 redis-cli get "integration-tests:engine:core-entity:workspace:$WS:data" > "$OUT/final-cache.json"
docker exec cw-twenty-pg-1 psql -qtAU postgres -d test -c "select \"displayName\" from core.workspace where id='$WS'" > "$OUT/final-db.txt"
docker exec cw-twenty-pg-1 psql -qtAU postgres -d test \
  -c "select data from pg_logical_slot_get_changes('cw', null, null, 'include-timestamp', 'on')" > "$OUT/wal.log"
echo "out: $OUT"
echo "final db: $(cat "$OUT/final-db.txt")  final cache: $(python3 -I -c 'import json,sys;d=open(sys.argv[1]).read().strip();print(json.loads(d).get("displayName") if d else None)' "$OUT/final-cache.json")"
