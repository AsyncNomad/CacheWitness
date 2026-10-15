#!/usr/bin/env bash
# Shared helpers for the isolated Twenty lab. Source this file.
NETNS=container:cw-twenty-pg-1

# Refuse to run if the lab namespace has any route off the internal subnet.
assert_isolated() {
  if docker exec cw-twenty-pg-1 awk 'NR>1 && $2=="00000000" {found=1} END {exit !found}' /proc/net/route; then
    echo "ABORT: cw-twenty network namespace has a default route; experiments must stay local" >&2
    exit 1
  fi
}

# Run a Node command inside the isolated namespace. npm and nx must not try
# the network (update checks, cloud cache, telemetry).
node_isolated() {
  docker run --rm --network "$NETNS" -u "$(id -u):$(id -g)" -e HOME=/tmp/h \
    -e NODE_ENV=test -e NX_NO_CLOUD=true -e NX_DAEMON=false -e NX_SKIP_NX_CACHE=false \
    -e npm_config_offline=true -e npm_config_update_notifier=false -e NO_UPDATE_NOTIFIER=1 \
    -e DO_NOT_TRACK=1 "$@"
}

# Wait until a TCP port answers inside the namespace.
wait_port() {
  until docker exec cw-twenty-pg-1 bash -c "</dev/tcp/127.0.0.1/$1" 2>/dev/null; do sleep 1; done
}
