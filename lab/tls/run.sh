#!/usr/bin/env bash
# TLS and authentication lab: Redis with a password and TLS, PostgreSQL with
# TLS, an HTTPS app with the hook, all inside the isolated lab namespace, with
# a throwaway private CA. Checks the happy path and the expected failures.
# Usage: lab/tls/run.sh      (needs the cw-twenty lab containers running)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT="$ROOT/lab/out/tls-lab"
C="$OUT/certs"
source "$ROOT/lab/twenty/isolation.sh"
assert_isolated
rm -rf "$OUT" && mkdir -p "$C"

# Private CA and a server certificate for localhost / 127.0.0.1.
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=cawit-lab-ca" -keyout "$C/ca.key" -out "$C/ca.pem" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=localhost" -keyout "$C/server.key" -out "$C/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\n' > "$C/san.ext"
openssl x509 -req -in "$C/server.csr" -CA "$C/ca.pem" -CAkey "$C/ca.key" -CAcreateserial -days 2 -extfile "$C/san.ext" -out "$C/server.pem" 2>/dev/null
chmod 644 "$C"/*

cleanup() { docker rm -f cw-tls-redis cw-tls-pg cw-tls-app > /dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup
docker run -d --name cw-tls-redis --network "$NETNS" -v "$C":/certs:ro redis:7.2 redis-server \
  --port 6391 --requirepass s3cret --tls-port 6392 --tls-cert-file /certs/server.pem \
  --tls-key-file /certs/server.key --tls-ca-cert-file /certs/ca.pem --tls-auth-clients no > /dev/null
docker run -d --name cw-tls-pg --network "$NETNS" -e POSTGRES_PASSWORD=postgres -v "$C":/certs:ro postgres:16 \
  bash -c 'cp /certs/server.pem /certs/server.key /tmp/ && chown postgres /tmp/server.* && chmod 600 /tmp/server.key &&
           exec docker-entrypoint.sh postgres -p 5441 -c ssl=on -c ssl_cert_file=/tmp/server.pem -c ssl_key_file=/tmp/server.key' > /dev/null
node_isolated -d --name cw-tls-app -e NODE_ENV=test -e PORT=8443 -e CERT=/certs/server.pem -e KEY=/certs/server.key \
  -e CAWIT_SYNC_URL=rediss://:s3cret@localhost:6392 -e NODE_EXTRA_CA_CERTS=/certs/ca.pem \
  -v "$C":/certs:ro -v "$ROOT/hooks/node":/hook:ro -v "$ROOT/lab/tls":/lab:ro node:22-slim node /lab/app.js > /dev/null
wait_port 6392; wait_port 8443
until docker exec cw-tls-pg pg_isready -q -p 5441 2>/dev/null; do sleep 1; done
sleep 1

cargo build --quiet --offline --release --manifest-path "$ROOT/Cargo.toml" -p cawit
cawit() {
  docker run --rm --network "$NETNS" -u "$(id -u):$(id -g)" -v "$ROOT/target/release/cawit":/usr/local/bin/cawit:ro \
    -e LAB_REDIS_PASSWORD=s3cret -e LAB_PG_PASSWORD=postgres \
    -v "$ROOT":"$ROOT":ro -w "$ROOT/lab/tls" ubuntu:24.04 cawit "$@"
}
variant() { # name, then sed expressions applied to the scenario
  local name=$1; shift
  sed "$@" "$ROOT/lab/tls/scenarios/all-tls.yaml" > "$OUT/$name.yaml"
  echo "$OUT/$name.yaml"
}
expect() { # expected exit code, label, scenario
  set +e; out=$(cawit verify "$3" 2>&1); code=$?; set -e
  if [ "$code" = "$1" ]; then echo "ok    $2 (exit $code)"; else echo "FAIL  $2: expected exit $1, got $code"; echo "$out" | tail -5; status=1; fi
  echo "$out" | grep -E "cawit:|tls_everywhere|runs" | head -3 | sed 's/^/        /'
}
status=0
expect 0 "rediss + AUTH, HTTPS, PostgreSQL TLS, private CA" "$ROOT/lab/tls/scenarios/all-tls.yaml"
expect 2 "wrong Redis password is an error" "$(variant badpass -e 's/:\${env.LAB_REDIS_PASSWORD}@/:wrong@/')"
expect 2 "unset password variable is an error" "$(variant noenv -e 's/LAB_REDIS_PASSWORD/LAB_UNSET_VARIABLE/')"
expect 2 "no CA: certificate verification fails" "$(variant noca -e '/ca_file/d' -e 's/^tls:$/tls: { insecure: false }/')"
expect 0 "insecure: skip verification without the CA" "$(variant insecure -e '/ca_file/d' -e 's/^tls:$/tls: { insecure: true }/')"
expect 1 "PostgreSQL without TLS: the check sees ssl=false" "$(variant nossl -e 's/sslmode=require/sslmode=disable/')"
exit $status
