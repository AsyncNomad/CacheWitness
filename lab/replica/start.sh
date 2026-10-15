#!/bin/bash
# Hot standby of the lab primary with a fixed apply delay, so reads from it
# lag commits by a known amount (replica-lag refill scenario).
set -euo pipefail
PGDATA=/var/lib/postgresql/data
rm -rf "${PGDATA:?}"/*
until pg_basebackup -h pg -U postgres -D "$PGDATA" -R -X stream -c fast; do sleep 1; done
echo "recovery_min_apply_delay = '${APPLY_DELAY:-50ms}'" >> "$PGDATA/postgresql.auto.conf"
chmod 700 "$PGDATA"
exec postgres -c hot_standby=on -c track_commit_timestamp=on
