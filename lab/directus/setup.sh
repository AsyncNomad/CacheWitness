#!/usr/bin/env bash
# Prepare the Directus checkout for the lab: fetch the pinned release, apply
# the CacheWitness test patch, install the API and its workspace dependencies
# only (not the admin app), and build them.
#
# THIS STEP USES THE NETWORK (git fetch from GitHub, pnpm install from npm),
# unlike every experiment script, which runs in the isolated lab namespace.
# Run it only with the maintainer's approval.
#
# Usage: lab/directus/setup.sh [checkout dir]   (default: ~/code/cw-targets/directus)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DIR=${1:-$HOME/code/cw-targets/directus}
TAG=v12.5.0
COMMIT=4f77933bf9551cac6b289576078783e189b8f4e3   # v12.5.0, 2026-10-07

if [ ! -d "$DIR/.git" ]; then
  mkdir -p "$DIR"
  git -C "$DIR" init -q
  git -C "$DIR" remote add origin https://github.com/directus/directus.git
fi
if [ "$(git -C "$DIR" rev-parse HEAD 2>/dev/null)" != "$COMMIT" ]; then
  git -C "$DIR" fetch -q --depth 1 origin tag "$TAG"
  git -C "$DIR" checkout -q "$COMMIT"
fi
PATCH="$ROOT/lab/directus/cawit.patch"
if git -C "$DIR" apply --check "$PATCH" 2>/dev/null; then
  git -C "$DIR" apply "$PATCH"
  echo "patch applied"
else
  git -C "$DIR" apply --reverse --check "$PATCH" && echo "patch already applied"
fi
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp/h -e COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
  -e npm_config_store_dir=/w/.pnpm-store -v "$DIR":/w -v "$ROOT/hooks/node":/hook:ro -w /w node:22-slim bash -c \
  'mkdir -p /tmp/h/bin && corepack enable --install-directory /tmp/h/bin && export PATH=/tmp/h/bin:$PATH &&
   pnpm install --frozen-lockfile --filter "@directus/api..." &&
   mkdir -p api/node_modules/cawit-hook && cp /hook/package.json /hook/index.js /hook/index.d.ts api/node_modules/cawit-hook/ &&
   pnpm --filter "@directus/api..." run build' \
  2>&1 | tail -15
echo "ready: $DIR ($TAG, patched)"
