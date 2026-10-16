#!/usr/bin/env bash
# Shared helpers for the Directus lab: database, bootstrap, two API servers.
# Source after setting ROOT, DX (checkout), OUT. Uses lab/twenty/isolation.sh.
# Server env: FIX, BUG, TTL, SYNC_TIMEOUT_S (hook hold timeout, 30), and HOOK=0
# to start without the cawit hook.
source "$ROOT/lab/twenty/isolation.sh"
TOKEN=cawit-lab-admin-token

dx_psql() { docker exec cw-twenty-pg-1 psql -qtAU postgres "$@"; }

# Create (RESET=1: recreate) the database and bootstrap Directus into it.
dx_prepare() {
  [ -n "${RESET:-}" ] && dx_psql -c "drop database if exists directus" > /dev/null
  [ "$(dx_psql -c "select 1 from pg_database where datname='directus'")" = 1 ] || dx_psql -c "create database directus" > /dev/null
  node_isolated "${DENV[@]}" -v "$DX":/w -w /w/api node:22-slim node dist/cli/run.js bootstrap > "$OUT/bootstrap.log" 2>&1 \
    || { echo "bootstrap failed (see $OUT/bootstrap.log)"; exit 3; }
}

DENV=(-e NODE_ENV=production -e DB_CLIENT=pg -e DB_HOST=127.0.0.1 -e DB_PORT=5432 -e DB_DATABASE=directus
  -e DB_USER=postgres -e DB_PASSWORD=postgres -e SECRET=cawit-lab-secret -e ADMIN_EMAIL=admin@example.com
  -e ADMIN_PASSWORD=cawit-lab -e ADMIN_TOKEN="$TOKEN" -e TELEMETRY=false -e SERVE_APP=false
  -e CACHE_ENABLED=true -e CACHE_STORE=redis -e REDIS=redis://127.0.0.1:6379 -e CACHE_AUTO_PURGE=true
  -e CACHE_TTL="${TTL:-5m}" -e CACHE_NAMESPACE=cawit-lab-directus -e CACHE_STATUS_HEADER=X-Cache-Status
  -e SYNCHRONIZATION_STORE=redis -e MESSENGER_STORE=redis -e RATE_LIMITER_ENABLED=false
  -e EXTENSIONS_PATH=/tmp/ext -e STORAGE_LOCAL_ROOT=/tmp/uploads -e LOG_LEVEL=warn)

dx_stop() {
  for i in 0 1; do
    docker logs "cw-directus-$i" > "$OUT/server-$i.log" 2>&1 || true
    docker rm -f "cw-directus-$i" > /dev/null 2>&1 || true
  done
}

# Start servers on 8055 and 8056, wait for them, and seed articles/1.
dx_start() {
  local hook=(-e CAWIT_SYNC_URL=redis://127.0.0.1:6379 -e CAWIT_ALLOW_PRODUCTION=1 -e CAWIT_SYNC_TIMEOUT_S="${SYNC_TIMEOUT_S:-30}")
  [ "${HOOK:-1}" = 0 ] && hook=()
  for i in 0 1; do
    node_isolated -d --name "cw-directus-$i" "${DENV[@]}" -e PORT=$((8055 + i)) -e PUBLIC_URL=http://127.0.0.1:$((8055 + i)) \
      "${hook[@]}" -e CW_FIX="${FIX:-none}" -e CW_BUG="${BUG:-}" -e CW_DDEL_MS=500 \
      -v "$DX":/w -w /w/api node:22-slim node dist/cli/run.js start > /dev/null
  done
  wait_port 8055; wait_port 8056
  node_isolated -v "$ROOT/lab/directus":/h:ro -e TOKEN="$TOKEN" node:22-slim node /h/seed.js
}
