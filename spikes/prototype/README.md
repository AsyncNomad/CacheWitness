# CacheWitness prototype (validation build)

Redis RESP2/RESP3 proxy + PostgreSQL logical decoding (test_decoding) reader that
detects cache entries inconsistent with the DB and labels the cause:
MISSING_INVALIDATION, INVALIDATION_FAILED, STALE_SET_RACE, INVALIDATE_BEFORE_COMMIT,
STALE_READ_AFTER_INVALIDATION (replica lag), STALE_FILL_AFTER_COMMIT, BOUNDED_BY_TTL (info).

## Run
1. Postgres with `wal_level=logical`; `select pg_create_logical_replication_slot('cw','test_decoding');`
   table `users(id int pk, name text, version int)` with `replica identity full`.
2. `cargo build --release && ./target/release/cachewitness`  (env: CW_LISTEN, CW_UPSTREAM, CW_PG,
   CW_TABLE, CW_PK, CW_PREFIX, CW_GRACE_MS, CW_TTL_OK_S, CW_OUT, CW_PASSTHROUGH)
3. Point the app's Redis URL at 127.0.0.1:6380. Violations are appended to violations.jsonl.

## Validation harness (harness/)
- `node harness.js`  : injects 6 bug types + 2 correct patterns with background load, writes truth.json
- `python3 eval.py`  : compares violations.jsonl with truth.json
- `node natural.js`  : textbook cache-aside under concurrency, no injected bugs; oracle = end-state compare

Prototype limits: one table/key mapping, JSON values only, polling test_decoding (not pgoutput streaming),
single host clock, no MULTI/EXEC or Lua awareness.
