// Phase 0 ground-truth harness.
//
// Runs textbook cache-aside (GET -> on miss SELECT -> SET; writes UPDATE then
// DEL after commit) through the proxy, and records from the application side
// what actually happened: which request issued each command, on which Redis
// connection, when it started and finished, and which row version it read or
// wrote. The proxy cannot see any of this; the analysis compares the proxy's
// verdicts against it.
//
// Env:
//   SECONDS (20)      run length
//   KEYS (200)        hot keys, ids 2001..2000+KEYS
//   WORKERS (32)      concurrent request loops
//   READ_RATIO (0.7)  share of requests that are reads
//   CONN_MODE (shared) shared: CONNS connections, workers round-robin over them
//                      per-worker: one connection per worker
//   CONNS (8)         connections in shared mode
//   FILL_DELAY_MS (5) max random delay between the SELECT and the SET
//   REPLICA_READS (0) share of cache-fill SELECTs sent to the lagging replica
//   REPLICA_PORT (5433)
//   PROXY / DIRECT    Redis URLs through and around the proxy
//   OUT (.)           output directory
const { Pool } = require('pg');
const { createClient } = require('redis');
const fs = require('fs');
const path = require('path');

const cfg = {
  seconds: +(process.env.SECONDS || 20),
  keys: +(process.env.KEYS || 200),
  workers: +(process.env.WORKERS || 32),
  readRatio: +(process.env.READ_RATIO || 0.7),
  connMode: process.env.CONN_MODE || 'shared',
  conns: +(process.env.CONNS || 8),
  fillDelayMs: +(process.env.FILL_DELAY_MS || 5),
  replicaReads: +(process.env.REPLICA_READS || 0),
  replicaPort: +(process.env.REPLICA_PORT || 5433),
  proxy: process.env.PROXY || 'redis://127.0.0.1:6380',
  direct: process.env.DIRECT || 'redis://127.0.0.1:6379',
  out: process.env.OUT || '.',
};
const BASE = 2001;

// Event times are on one continuous timeline: performance.now() (monotonic,
// sub-millisecond) plus the wall-clock offset measured at startup. The wall
// clock itself steps and slews (WSL2 steps it by seconds every few tens of
// seconds, then slews), and PostgreSQL's commit timestamps follow the wall
// clock. So the harness also logs (timeline, Date.now()) samples about every
// millisecond, and analyze.py derives the wall-clock offset at any moment from
// them to move commit timestamps onto this timeline.
const measureOffset = () => {
  let best = -Infinity;
  const until = performance.now() + 20;
  while (performance.now() < until) best = Math.max(best, Date.now() - performance.now());
  return best;
};
const OFFSET0 = measureOffset();
const clockLog = [];
let lastSample = -Infinity;
const sampleWall = (t) => {
  if (t - lastSample >= 1) {
    clockLog.push([+t.toFixed(3), Date.now()]);
    lastSample = t;
  }
};
setInterval(() => sampleWall(performance.now() + OFFSET0), 1).unref();
const now = () => {
  const t = performance.now() + OFFSET0;
  sampleWall(t);
  return t;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const enc = (r) => JSON.stringify({ id: r.id, name: r.name, version: r.version });

const events = [];
const ev = (e) => events.push(e);

(async () => {
  const pool = new Pool({ host: '127.0.0.1', user: 'postgres', database: 'app', max: cfg.workers + 4 });
  const replica = cfg.replicaReads > 0
    ? new Pool({ host: '127.0.0.1', port: cfg.replicaPort, user: 'postgres', database: 'app', max: cfg.workers + 4 })
    : null;

  // Connect one at a time so the proxy assigns connection ids in this order
  // (the prototype numbers accepted connections from 1).
  const nConns = cfg.connMode === 'per-worker' ? cfg.workers : cfg.conns;
  const clients = [];
  for (let i = 0; i < nConns; i++) {
    const c = createClient({ url: cfg.proxy });
    await c.connect();
    clients.push(c);
  }

  let reqSeq = 0;
  const stop = Date.now() + cfg.seconds * 1000;
  await Promise.all(Array.from({ length: cfg.workers }, async (_, w) => {
    const ci = w % nConns;
    const c = clients[ci];
    const conn = ci + 1; // proxy connection id
    while (Date.now() < stop) {
      const r = ++reqSeq;
      const id = BASE + Math.floor(Math.random() * cfg.keys);
      const key = `user:${id}`;
      if (Math.random() < cfg.readRatio) {
        let s = now();
        const got = await c.get(key);
        ev({ t: 'get', r, w, conn, k: id, s, e: now(), hit: got !== null, v: got ? JSON.parse(got).version : null });
        if (got !== null) continue;
        const tgt = replica && Math.random() < cfg.replicaReads ? 'replica' : 'primary';
        s = now();
        const row = (await (tgt === 'replica' ? replica : pool).query(
          'select id, name, version, xmin::text::bigint as xmin from users where id = $1', [id])).rows[0];
        ev({ t: 'sel', r, w, k: id, s, e: now(), tgt, v: row.version, xmin: +row.xmin });
        await sleep(Math.random() * cfg.fillDelayMs);
        s = now();
        await c.set(key, enc(row), { EX: 3600 });
        ev({ t: 'set', r, w, conn, k: id, s, e: now(), v: row.version });
      } else {
        let s = now();
        const row = (await pool.query(
          'update users set name = md5(random()::text), version = version + 1 where id = $1 returning version, xmin::text::bigint as xid', [id])).rows[0];
        ev({ t: 'upd', r, w, k: id, s, e: now(), v: row.version, xid: +row.xid });
        s = now();
        const n = await c.del(key);
        ev({ t: 'del', r, w, conn, k: id, s, e: now(), n });
      }
    }
  }));

  // Quiesce, then record the end state straight from Redis and the primary.
  await sleep(2000);
  const direct = createClient({ url: cfg.direct });
  await direct.connect();
  const endState = [];
  for (let id = BASE; id < BASE + cfg.keys; id++) {
    const v = await direct.get(`user:${id}`);
    const db = (await pool.query('select version from users where id = $1', [id])).rows[0].version;
    endState.push({ k: id, cache: v ? JSON.parse(v).version : null, db });
  }

  // Commit timestamps of every write, for the commit-time vs visibility analysis.
  const xids = events.filter((e) => e.t === 'upd').map((e) => e.xid);
  const commitTs = {};
  for (let i = 0; i < xids.length; i += 5000) {
    const chunk = xids.slice(i, i + 5000);
    const rows = (await pool.query(
      `select x::text as xid, extract(epoch from pg_xact_commit_timestamp(x::text::xid)) * 1000 as ts
         from unnest($1::bigint[]) as x`, [chunk])).rows;
    for (const row of rows) commitTs[row.xid] = row.ts === null ? null : +row.ts;
  }

  fs.writeFileSync(path.join(cfg.out, 'truth.jsonl'), events.map((e) => JSON.stringify(e)).join('\n') + '\n');
  fs.writeFileSync(path.join(cfg.out, 'end_state.json'), JSON.stringify(endState));
  fs.writeFileSync(path.join(cfg.out, 'commit_ts.json'), JSON.stringify(commitTs));
  fs.writeFileSync(path.join(cfg.out, 'run.json'), JSON.stringify({ cfg, events: events.length }, null, 1));
  fs.writeFileSync(path.join(cfg.out, 'clock.json'), JSON.stringify(clockLog));
  const count = (t) => events.filter((e) => e.t === t).length;
  console.log(JSON.stringify({ gets: count('get'), selects: count('sel'), sets: count('set'), updates: count('upd'), dels: count('del'),
    stale_at_end: endState.filter((s) => s.cache !== null && s.cache !== s.db).length }));
  await direct.quit();
  await Promise.all(clients.map((c) => c.quit()));
  await pool.end();
  if (replica) await replica.end();
})().catch((e) => { console.error(e); process.exit(1); });
