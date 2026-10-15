#!/usr/bin/env bash
# Race experiment matrix on Twenty: server count x write interval.
# Usage: lab/twenty/race-matrix.sh <twenty checkout>
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SECS=${SECS:-30}
LIST="$ROOT/lab/out/race-matrix-$(date +%Y%m%d-%H%M%S).txt"
first=1
for n in 1 2; do
  for interval in 100 20 0; do
    reset=""; [ $first = 1 ] || reset=1; first=0
    dir=$(NSERVERS=$n SKIP_RESET=$reset "$ROOT/lab/twenty/race.sh" "$1" SECONDS="$SECS" WRITE_INTERVAL_MS="$interval" \
      | sed -n 's/^out: //p')
    python3 -I "$ROOT/lab/twenty/race_analyze.py" "$dir" > "$dir/race-analysis.txt"
    echo "$n $interval $dir" | tee -a "$LIST"
  done
done
echo "list: $LIST"
