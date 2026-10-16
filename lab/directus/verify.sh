#!/usr/bin/env bash
# Run a cawit verify scenario against two Directus API servers with a chosen fix.
# Usage: [FIX=none|ddel|gen] [N=20] [RESET=1] [SCENARIO=stale-set] lab/directus/verify.sh <directus checkout>
# Needs lab/directus/setup.sh (patched and built). Shares the isolated lab
# network of lab/twenty (same Postgres instance, database `directus`; same
# Redis). RESET=1 drops and re-bootstraps the database. Exit code is cawit's.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DX=$(cd "${1:?directus checkout}" && pwd)
FIX=${FIX:-none}; N=${N:-20}; SCENARIO=${SCENARIO:-stale-set}
OUT="$ROOT/lab/out/directus-$SCENARIO-$FIX-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
TOKEN=cawit-lab-admin-token
source "$ROOT/lab/twenty/isolation.sh"
assert_isolated

mkdir -p "$DX/api/node_modules/cawit-hook"
cp "$ROOT"/hooks/node/{package.json,index.js,index.d.ts} "$DX/api/node_modules/cawit-hook/"
cargo build --quiet --offline --release --manifest-path "$ROOT/Cargo.toml" -p cawit

psql() { docker exec cw-twenty-pg-1 psql -qtAU postgres "$@"; }
if [ -n "${RESET:-}" ]; then
  psql -c "drop database if exists directus" > /dev/null
fi
[ "$(psql -c "select 1 from pg_database where datname='directus'")" = 1 ] || psql -c "create database directus" > /dev/null

DENV=(-e NODE_ENV=production -e DB_CLIENT=pg -e DB_HOST=127.0.0.1 -e DB_PORT=5432 -e DB_DATABASE=directus
  -e DB_USER=postgres -e DB_PASSWORD=postgres -e SECRET=cawit-lab-secret -e ADMIN_EMAIL=admin@example.com
  -e ADMIN_PASSWORD=cawit-lab -e ADMIN_TOKEN="$TOKEN" -e TELEMETRY=false -e SERVE_APP=false
  -e CACHE_ENABLED=true -e CACHE_STORE=redis -e REDIS=redis://127.0.0.1:6379 -e CACHE_AUTO_PURGE=true
  -e CACHE_TTL=5m -e CACHE_NAMESPACE=cawit-lab-directus -e CACHE_STATUS_HEADER=X-Cache-Status
  -e SYNCHRONIZATION_STORE=redis -e MESSENGER_STORE=redis -e RATE_LIMITER_ENABLED=false
  -e EXTENSIONS_PATH=/tmp/ext -e STORAGE_LOCAL_ROOT=/tmp/uploads -e LOG_LEVEL=warn)
node_isolated "${DENV[@]}" -v "$DX":/w -w /w/api node:22-slim node dist/cli/run.js bootstrap > "$OUT/bootstrap.log" 2>&1 \
  || { echo "bootstrap failed (see $OUT/bootstrap.log)"; exit 3; }

cleanup() {
  for i in 0 1; do
    docker logs "cw-directus-$i" > "$OUT/server-$i.log" 2>&1 || true
    docker rm -f "cw-directus-$i" > /dev/null 2>&1 || true
  done
}
trap cleanup EXIT
for i in 0 1; do
  node_isolated -d --name "cw-directus-$i" "${DENV[@]}" -e PORT=$((8055 + i)) -e PUBLIC_URL=http://127.0.0.1:$((8055 + i)) \
    -e CAWIT_SYNC_URL=redis://127.0.0.1:6379 -e CAWIT_ALLOW_PRODUCTION=1 -e CW_FIX="$FIX" -e CW_DDEL_MS=500 \
    -v "$DX":/w -w /w/api node:22-slim node dist/cli/run.js start > /dev/null
done
wait_port 8055; wait_port 8056
node_isolated -v "$ROOT/lab/directus":/h:ro -e TOKEN="$TOKEN" node:22-slim node /h/seed.js
echo "servers up (fix=$FIX)"
set +e
docker run --rm --network "$NETNS" -u "$(id -u):$(id -g)" -e CAWIT_TOKEN="$TOKEN" \
  -v "$ROOT/target/release/cawit":/usr/local/bin/cawit:ro -v "$ROOT/lab/directus/scenarios":/s:ro -v "$OUT":/out \
  ubuntu:24.04 cawit verify "/s/$SCENARIO.yaml" --runs "$N" --out /out/report.json --junit /out/junit.xml | tee "$OUT/verify.out"
code=${PIPESTATUS[0]}
set -e
echo "exit $code  out: $OUT"
exit "$code"
