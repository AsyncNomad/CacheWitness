#!/usr/bin/env bash
# Deterministic stale-set scenario on Twenty with a chosen fix.
# Usage: [FIX=none|vcas|lease] [LEASE_RETRY=1] [N=20] [RESET=1] lab/twenty/sync.sh <twenty checkout>
# Needs refill-delay.patch applied; RESET=1 rebuilds (database:reset builds first).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TWENTY=$(cd "${1:?twenty checkout}" && pwd)
FIX=${FIX:-none}; N=${N:-20}
OUT="$ROOT/lab/out/sync-$FIX-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
WS=20202020-1c25-4d02-bf25-6aeccf7ea419
TOKEN=$(python3 -I -c 'import json,sys;print(json.load(open(sys.argv[1]))["APPLE_JANE_ADMIN_ACCESS_TOKEN"])' \
  "$TWENTY/packages/twenty-server/test/integration/constants/test-tokens.json")
source "$ROOT/lab/twenty/isolation.sh"
assert_isolated
if [ -n "${RESET:-}" ]; then
  echo "reset database (and build)"
  node_isolated -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm bash -c \
    "NODE_OPTIONS='--max-old-space-size=8192' npx nx database:reset" > "$OUT/reset.log" 2>&1
fi
seeded=$(docker exec cw-twenty-pg-1 psql -qtAU postgres -d test -c "select count(*) from core.workspace where id='$WS'" 2>/dev/null || echo 0)
[ "$seeded" = "1" ] || { echo "seeded workspace missing (see $OUT/reset.log)"; exit 3; }
cleanup() {
  for i in 0 1; do
    docker logs "cw-twenty-server-$i" > "$OUT/server-$i.log" 2>&1 || true
    docker rm -f "cw-twenty-server-$i" > /dev/null 2>&1 || true
  done
}
trap cleanup EXIT
for i in 0 1; do
  node_isolated -d --name "cw-twenty-server-$i" -e NODE_PORT=$((3000 + i)) -e CW_SYNC=1 -e CW_FIX="$FIX" -e CW_LEASE_RETRY="${LEASE_RETRY:-0}" \
    -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm node dist/main > /dev/null
done
wait_port 3000; wait_port 3001
echo "servers up (fix=$FIX)"
node_isolated -v "$ROOT/lab/twenty":/h:ro -v "$ROOT/lab/truth/node_modules":/nm:ro -v "$OUT":/out -w /out \
  -e NODE_PATH=/nm -e SERVERS=http://127.0.0.1:3000,http://127.0.0.1:3001 -e TOKEN="$TOKEN" \
  -e N="$N" -e CW_FIX="$FIX" -e OUT=/out node:24.16.0-bookworm node /h/sync-scenario.js | tee "$OUT/sync.out"
echo "out: $OUT"
