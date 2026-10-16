#!/usr/bin/env bash
# Run a cawit verify scenario against two Twenty servers with a chosen fix.
# Usage: [FIX=none|vcas|lease] [LEASE_RETRY=1] [N=20] [RESET=1] [SCENARIO=stale-set] \
#          lab/twenty/verify.sh <twenty checkout>
# Needs refill-delay.patch applied; RESET=1 rebuilds and reseeds (database:reset
# builds first). Everything, including the cawit binary, runs inside the
# isolated lab namespace. Exit code is cawit's (0 pass, 1 fail, 2 inconclusive).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TWENTY=$(cd "${1:?twenty checkout}" && pwd)
FIX=${FIX:-none}; N=${N:-20}; SCENARIO=${SCENARIO:-stale-set}
OUT="$ROOT/lab/out/verify-$SCENARIO-$FIX$([ "${LEASE_RETRY:-0}" = 1 ] && echo -retry)-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
WS=20202020-1c25-4d02-bf25-6aeccf7ea419
TOKEN=$(python3 -I -c 'import json,sys;print(json.load(open(sys.argv[1]))["APPLE_JANE_ADMIN_ACCESS_TOKEN"])' \
  "$TWENTY/packages/twenty-server/test/integration/constants/test-tokens.json")
source "$ROOT/lab/twenty/isolation.sh"
assert_isolated

# Install the hook the way `npm install hooks/node` would, without the network.
mkdir -p "$TWENTY/node_modules/cawit-hook"
cp "$ROOT"/hooks/node/{package.json,index.js,index.d.ts} "$TWENTY/node_modules/cawit-hook/"
cargo build --quiet --offline --release --manifest-path "$ROOT/Cargo.toml" -p cawit

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
  node_isolated -d --name "cw-twenty-server-$i" -e NODE_PORT=$((3000 + i)) \
    -e CAWIT_SYNC_URL=redis://127.0.0.1:6379 -e CW_FIX="$FIX" -e CW_LEASE_RETRY="${LEASE_RETRY:-0}" \
    -v "$TWENTY":/w -w /w/packages/twenty-server node:24.16.0-bookworm node dist/main > /dev/null
done
wait_port 3000; wait_port 3001
echo "servers up (fix=$FIX lease_retry=${LEASE_RETRY:-0})"
set +e
docker run --rm --network "$NETNS" -u "$(id -u):$(id -g)" -e CAWIT_TOKEN="$TOKEN" \
  -v "$ROOT/target/release/cawit":/usr/local/bin/cawit:ro -v "$ROOT/lab/twenty/scenarios":/s:ro -v "$OUT":/out \
  ubuntu:24.04 cawit verify "/s/$SCENARIO.yaml" --runs "$N" --out /out/report.json --junit /out/junit.xml | tee "$OUT/verify.out"
code=${PIPESTATUS[0]}
set -e
echo "exit $code  out: $OUT"
exit "$code"
