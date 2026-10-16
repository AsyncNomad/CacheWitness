#!/usr/bin/env bash
# Regenerate a lab patch from a modified target checkout, keeping its license
# header (lab/<target>/PATCH-HEADER.txt). Never publish a lab patch without it:
# the patches contain excerpts of the target's code under the target's license.
# Usage: lab/make-patch.sh twenty|directus <checkout>
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
case "${1:?target}" in
  twenty) OUT="$ROOT/lab/twenty/refill-delay.patch" ;;
  directus) OUT="$ROOT/lab/directus/cawit.patch" ;;
  *) echo "unknown target $1" >&2; exit 2 ;;
esac
DIR=$(cd "${2:?checkout}" && pwd)
{ cat "$ROOT/lab/$1/PATCH-HEADER.txt"; git -C "$DIR" diff; } > "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
echo "wrote $OUT"
