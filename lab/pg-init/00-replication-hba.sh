#!/bin/bash
# Let the replica service stream from this primary (lab only: trust auth).
echo "host replication all all trust" >> "$PGDATA/pg_hba.conf"
