#!/usr/bin/env bash
# Phase 0 experiment matrix: connection layout x read ratio x repeats.
# Each cell is a 20s truth run through the prototype, then analyze.py.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REPEATS=${REPEATS:-2}
SECS=${SECS:-20}
LIST="$ROOT/lab/out/matrix-$(date +%Y%m%d-%H%M%S).txt"
for mode in shared per-worker; do
  for ratio in 0.7 0.95 0.99; do
    for rep in $(seq 1 "$REPEATS"); do
      dir=$("$ROOT/lab/repro.sh" truth SECONDS="$SECS" READ_RATIO="$ratio" CONN_MODE="$mode" | sed -n 's/^out: //p')
      python3 -I "$ROOT/lab/truth/analyze.py" "$dir" > "$dir/analysis.txt"
      echo "$mode $ratio $rep $dir" | tee -a "$LIST"
    done
  done
done
echo "list: $LIST"
