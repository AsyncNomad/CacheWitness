#!/usr/bin/env bash
# Run a cawit verify scenario against two Directus API servers with a chosen fix.
# Usage: [FIX=none|ddel|gen|retry] [BUG=early-purge] [TTL=5m] [N=20] [RESET=1] [SCENARIO=stale-set] lab/directus/verify.sh <directus checkout>
# Needs lab/directus/setup.sh (patched and built). Shares the isolated lab
# network of lab/twenty (same Postgres instance, database `directus`; same
# Redis). RESET=1 drops and re-bootstraps the database. Exit code is cawit's.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DX=$(cd "${1:?directus checkout}" && pwd)
FIX=${FIX:-none}; N=${N:-20}; SCENARIO=${SCENARIO:-stale-set}
OUT="$ROOT/lab/out/directus-$SCENARIO-$FIX${BUG:+-bug-$BUG}-ttl${TTL:-5m}-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
source "$ROOT/lab/directus/common.sh"
assert_isolated

mkdir -p "$DX/api/node_modules/cawit-hook"
cp "$ROOT"/hooks/node/{package.json,index.js,index.d.ts} "$DX/api/node_modules/cawit-hook/"
cargo build --quiet --offline --release --manifest-path "$ROOT/Cargo.toml" -p cawit

dx_prepare
trap dx_stop EXIT
dx_start
echo "servers up (fix=$FIX)"
set +e
docker run --rm --network "$NETNS" -u "$(id -u):$(id -g)" -e CAWIT_TOKEN="$TOKEN" \
  -v "$ROOT/target/release/cawit":/usr/local/bin/cawit:ro -v "$ROOT/lab/directus/scenarios":/s:ro -v "$OUT":/out \
  ubuntu:24.04 cawit verify "/s/$SCENARIO.yaml" --runs "$N" --out /out/report.json --junit /out/junit.xml | tee "$OUT/verify.out"
code=${PIPESTATUS[0]}
set -e
echo "exit $code  out: $OUT"
exit "$code"
