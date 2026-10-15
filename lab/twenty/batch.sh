#!/usr/bin/env bash
# Run a list of race experiments. Each line of the plan file:
#   <label> <SECONDS> <space-separated KEY=VALUE for race.sh and race.js>
# race.sh options (NSERVERS, REFILL_DELAY_MS, JITTER, REFILL_MODE, AMP_DELAY_MS)
# and race.js options (BURST, BURST_PAUSE_MS, ...) can be mixed; the first
# group is passed as env, the rest as race.js arguments.
# Usage: lab/twenty/batch.sh <twenty checkout> <plan file>
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LIST="$ROOT/lab/out/batch-$(date +%Y%m%d-%H%M%S).txt"
SH_KEYS="NSERVERS REFILL_DELAY_MS JITTER REFILL_MODE AMP_DELAY_MS PG_DELAY_MS SKIP_RESET"
while read -r label secs rest; do
  [ -z "${label:-}" ] || [ "${label:0:1}" = "#" ] && continue
  envs=(SKIP_RESET=1 NSERVERS=2); args=(SECONDS="$secs")
  for kv in $rest; do
    if [[ " $SH_KEYS " == *" ${kv%%=*} "* ]]; then envs+=("$kv"); else args+=("$kv"); fi
  done
  out=$(env "${envs[@]}" "$ROOT/lab/twenty/race.sh" "$1" "${args[@]}" 2>&1) || true
  dir=$(echo "$out" | sed -n 's/^out: //p')
  [ -n "$dir" ] && python3 -I "$ROOT/lab/twenty/race_analyze.py" "$dir" > "$dir/race-analysis.txt"
  echo "$label ${dir:-FAILED} $(echo "$out" | grep -E '^(OK|INVALID)' | head -1)" | tee -a "$LIST"
done < "$2"
echo "list: $LIST"
