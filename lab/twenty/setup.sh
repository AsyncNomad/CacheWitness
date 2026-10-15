#!/usr/bin/env bash
# Prepare the Twenty checkout for the lab: fetch the pinned commit, apply the
# CacheWitness test patch, and install dependencies.
#
# THIS STEP USES THE NETWORK (git fetch from GitHub, yarn install from npm),
# unlike every experiment script, which runs in the isolated lab namespace.
# Run it only with the maintainer's approval.
#
# Usage: lab/twenty/setup.sh [checkout dir]   (default: ~/code/cw-targets/twenty)
# Then build + seed with: RESET=1 FIX=none N=1 lab/twenty/sync.sh <checkout dir>
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DIR=${1:-$HOME/code/cw-targets/twenty}
COMMIT=c3ba1401f45dbc35bf5ed3d48c4ddc2e58f2bb15   # 2026-10-14, used in phase 1

if [ ! -d "$DIR/.git" ]; then
  mkdir -p "$DIR"
  git -C "$DIR" init -q
  git -C "$DIR" remote add origin https://github.com/twentyhq/twenty.git
fi
if [ "$(git -C "$DIR" rev-parse HEAD 2>/dev/null)" != "$COMMIT" ]; then
  git -C "$DIR" fetch -q --depth 1 origin "$COMMIT"
  git -C "$DIR" checkout -q FETCH_HEAD
fi
# Apply the test patch (sync point, refill delays, candidate fixes) once.
if git -C "$DIR" apply --check "$ROOT/lab/twenty/refill-delay.patch" 2>/dev/null; then
  git -C "$DIR" apply "$ROOT/lab/twenty/refill-delay.patch"
  echo "patch applied"
else
  git -C "$DIR" apply --reverse --check "$ROOT/lab/twenty/refill-delay.patch" && echo "patch already applied"
fi
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp/h -e YARN_CACHE_FOLDER=/w/.yarn-cache \
  -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 -v "$DIR":/w -w /w node:24.16.0-bookworm bash -c \
  'mkdir -p /tmp/h/bin && corepack enable --install-directory /tmp/h/bin && export PATH=/tmp/h/bin:$PATH && yarn install' \
  | tail -3
echo "ready: $DIR (commit ${COMMIT:0:8}, patched)"
