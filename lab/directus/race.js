// Natural stale-set frequency on Directus (no hook, no fix): readers keep
// reading articles/1 through both servers while one writer updates it now
// and then. After each write settles, a check read through server A is
// compared with the database: a different value means the cache kept a
// stale response (it stays until the next write purges, or CACHE_TTL).
//
// A stale-set needs a refill in flight when a write commits and purges. With
// writes spaced out the cache is warm again before each write and every read
// is a hit, so the race cannot happen; BURST=k sends k writes back to back
// (several edits in quick succession), so the first purge's misses are still
// refilling when the next write lands.
//
// Env: SERVERS (http://127.0.0.1:8055,http://127.0.0.1:8056)  TOKEN
//   READERS (2, per server)  THINK_MS (50, mean pause between a reader's
//   requests, uniform 0..2x)  WRITES (300 rounds)  BURST (1 write per round)
//   SETTLE_MS (300)  GAP_MS (200)
//   OUT (.)  LABEL
const fs = require('fs');
const path = require('path');
const { Client } = require('pg');

const cfg = {
  servers: (process.env.SERVERS || 'http://127.0.0.1:8055,http://127.0.0.1:8056').split(','),
  token: process.env.TOKEN,
  readers: +(process.env.READERS || 2),
  thinkMs: +(process.env.THINK_MS ?? 50),
  writes: +(process.env.WRITES || 300),
  burst: +(process.env.BURST || 1),
  settleMs: +(process.env.SETTLE_MS || 300),
  gapMs: +(process.env.GAP_MS || 200),
  out: process.env.OUT || '.',
  label: process.env.LABEL || '',
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const headers = { authorization: `Bearer ${cfg.token}`, 'content-type': 'application/json' };

async function read(base) {
  const res = await fetch(`${base}/items/articles/1`, { headers });
  const body = await res.json();
  return { title: body.data?.title, cache: res.headers.get('x-cache-status') };
}

(async () => {
  const pg = new Client({ host: '127.0.0.1', user: 'postgres', password: 'postgres', database: 'directus' });
  await pg.connect();
  let stop = false;
  let reads = 0;
  let readErrors = 0;
  const readers = [];
  for (const base of cfg.servers) {
    for (let r = 0; r < cfg.readers; r++) {
      readers.push((async () => {
        while (!stop) {
          try { await read(base); reads++; } catch { readErrors++; }
          if (cfg.thinkMs > 0) await sleep(Math.random() * 2 * cfg.thinkMs);
        }
      })());
    }
  }
  const runId = Date.now().toString(36);
  const t0 = Date.now();
  const results = [];
  for (let i = 1; i <= cfg.writes; i++) {
    let value;
    for (let j = 1; j <= cfg.burst; j++) {
      value = `race-${runId}-${i}-${j}`;
      const writer = cfg.servers[(i + j) % cfg.servers.length];
      const res = await fetch(`${writer}/items/articles/1`, { method: 'PATCH', headers, body: JSON.stringify({ title: value }) });
      if (!res.ok) throw new Error(`write ${i}.${j}: HTTP ${res.status}`);
    }
    await sleep(cfg.settleMs);
    const served = await read(cfg.servers[0]);
    const db = (await pg.query('select title from articles where id = 1')).rows[0].title;
    const stale = served.title !== db;
    results.push({ i, value, db, served: served.title, cache: served.cache, stale });
    if (stale) console.log(JSON.stringify(results[results.length - 1]));
    await sleep(cfg.gapMs);
  }
  stop = true;
  await Promise.all(readers);
  const seconds = (Date.now() - t0) / 1000;
  const staleCount = results.filter((r) => r.stale).length;
  const summary = {
    label: cfg.label, readers_per_server: cfg.readers, think_ms: cfg.thinkMs, rounds: cfg.writes, burst: cfg.burst,
    reads_per_s: +(reads / seconds).toFixed(1), read_errors: readErrors,
    stale: staleCount, stale_rate: +(staleCount / cfg.writes).toFixed(4),
    stale_were_cache_hits: results.filter((r) => r.stale && r.cache === 'HIT').length,
  };
  fs.writeFileSync(path.join(cfg.out, `race-${cfg.label || runId}.json`), JSON.stringify({ summary, results }, null, 1));
  console.log('summary', JSON.stringify(summary));
  await pg.end();
})().catch((e) => { console.error(e); process.exit(1); });
