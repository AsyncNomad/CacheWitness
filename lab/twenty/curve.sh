#!/usr/bin/env bash
# Delay-probability curves on Twenty: per burst of 8 concurrent renames (two
# servers, 600ms pause), how often does the cache keep an outdated workspace?
# Two ways to slow the refill, compared at the same delays:
#   code  a sleep inside Twenty's refill path (refill-delay.patch): a real slow refill
#   amp   the proxy amplifier holding the refill write (no app change)
# Usage: lab/twenty/curve.sh <twenty checkout>   (env: SECS per point, DELAYS, JITTER, MODES)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SECS=${SECS:-360}
DELAYS=${DELAYS:-"5 20 50 100"}
JITTER=${JITTER:-1}
LIST="$ROOT/lab/out/curve-$(date +%Y%m%d-%H%M%S).txt"
for delay in $DELAYS; do
  for mode in ${MODES:-code amp}; do
    if [ $mode = code ]; then knob=(REFILL_DELAY_MS=$delay); else knob=(AMP_DELAY_MS=$delay); fi
    out=$(env SKIP_RESET=1 NSERVERS=2 JITTER=$JITTER "${knob[@]}" "$ROOT/lab/twenty/race.sh" "$1" \
      SECONDS="$SECS" BURST=8 BURST_PAUSE_MS=600 2>&1) || true
    dir=$(echo "$out" | sed -n 's/^out: //p')
    [ -n "$dir" ] && python3 -I "$ROOT/lab/twenty/race_analyze.py" "$dir" > "$dir/race-analysis.txt"
    echo "$mode $delay $JITTER ${dir:-FAILED} $(echo "$out" | grep -E '^(OK|INVALID)' | head -1)" | tee -a "$LIST"
  done
done
echo "list: $LIST"
