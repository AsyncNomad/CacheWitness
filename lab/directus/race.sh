#!/usr/bin/env bash
# Natural stale-set frequency on two Directus servers without the hook or a
# fix (race.js). Runs each condition twice: results from probabilistic runs
# vary between runs (phase 1, Twenty), so one run is not a rate.
# Usage: [WRITES=300] [CONDITIONS="..."] [RESET=1] lab/directus/race.sh <directus checkout>
# A condition is "label readers-per-server think-ms burst". Default: spaced
# writes and bursts of 2 and 5, each with light (2 readers/server, ~50 ms
# think time) and heavy (8 readers/server, no think time) readers.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DX=$(cd "${1:?directus checkout}" && pwd)
OUT="$ROOT/lab/out/directus-race-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
HOOK=0 FIX=none
source "$ROOT/lab/directus/common.sh"
assert_isolated
dx_prepare
trap dx_stop EXIT
dx_start
CONDITIONS=${CONDITIONS:-"light 2 50 1|heavy 8 0 1|light-b2 2 50 2|heavy-b2 8 0 2|light-b5 2 50 5|heavy-b5 8 0 5"}
IFS='|' read -ra CONDS <<< "$CONDITIONS"
for cond in "${CONDS[@]}"; do
  set -- $cond
  for rep in 1 2; do
    node_isolated -v "$ROOT/lab/directus":/h:ro -v "$ROOT/lab/truth/node_modules":/nm:ro -v "$OUT":/out -w /out \
      -e NODE_PATH=/nm -e TOKEN="$TOKEN" -e READERS="$2" -e THINK_MS="$3" -e BURST="$4" -e WRITES="${WRITES:-300}" \
      -e OUT=/out -e LABEL="$1-$rep" node:22-slim node /h/race.js | grep '^summary'
  done
done
echo "out: $OUT"
