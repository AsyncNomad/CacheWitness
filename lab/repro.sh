#!/usr/bin/env bash
# Reproduce the prototype's validation runs (PLAN 5.1) against the lab.
# Usage: lab/repro.sh injected|natural|truth [KEY=VALUE env for the harness...]
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
MODE=${1:?injected|natural|truth}; shift || true
OUT="$ROOT/lab/out/$MODE-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
BIN="$ROOT/spikes/prototype/cachewitness/target/release/cachewitness"
HARNESS="$ROOT/spikes/prototype/harness"
[ "$MODE" = truth ] && HARNESS="$ROOT/lab/truth"

# Fresh state: rows back to version 1, empty cache, a new slot with no backlog.
# Separate -c flags: each runs in its own transaction (slot creation must not follow writes).
docker exec cw-lab-pg-1 psql -qU postgres -d app \
  -c "select pg_drop_replication_slot('cw') from pg_replication_slots where slot_name='cw'" \
  -c "update users set name=md5(id::text), version=1" \
  -c "select 1 from pg_create_logical_replication_slot('cw','test_decoding')" >/dev/null
docker exec cw-lab-redis-1 redis-cli flushall >/dev/null

(cd "$OUT" && CW_OUT="$OUT/violations.jsonl" exec "$BIN" 2>"$OUT/proxy.log") &
PROXY_PID=$!
trap 'kill $PROXY_PID 2>/dev/null || true' EXIT
sleep 1

run_node() {
  docker run --rm --network host -u "$(id -u):$(id -g)" \
    -v "$HARNESS":/h:ro -v "$OUT":/out -w /out "$@" node:22-slim node "/h/$SCRIPT"
}
case "$MODE" in
  injected) SCRIPT=harness.js run_node "${@/#/-e}" | tee "$OUT/harness.out"
            sleep 1
            (cd "$OUT" && python3 -I "$HARNESS/eval.py" violations.jsonl) | tee "$OUT/eval.out" ;;
  natural)  SCRIPT=natural.js run_node "${@/#/-e}" | tee "$OUT/natural.out"
            sleep 1 ;;
  truth)    SCRIPT=run.js run_node -e OUT=/out "${@/#/-e}" | tee "$OUT/truth.out"
            sleep 1 ;;
esac
kill $PROXY_PID; wait $PROXY_PID 2>/dev/null || true
tail -1 "$OUT/proxy.log" > "$OUT/stats.out" || true
echo "out: $OUT"
cat "$OUT/stats.out"
