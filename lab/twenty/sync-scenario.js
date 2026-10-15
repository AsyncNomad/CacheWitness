// Deterministic stale-set scenario on Twenty (needs refill-delay.patch with
// CW_SYNC=1 on the servers). Each iteration forces one interleaving:
//
//   1. rename on server A  -> A commits and invalidates (clears A's memo too)
//   2. arm the sync point, read on server A -> A misses, reads the DB (old
//      value), stops at the sync point and signals cw:sync:reached
//   3. rename on server B  -> B commits the new value and invalidates
//   4. release A           -> A writes what it read
//   5. compare the cached workspace with the database
//   6. read once more on A and check the cache refills with the new value
//      (a fix must not stop caching altogether)
//
// Without a fix A writes the old value after B's invalidation: stale, every
// time. The verdict is the final state; "reached" doubles as the
// manipulation check (the interleaving really happened).
//
// Env: SERVERS (two base URLs)  TOKEN  N (20)  OUT (.)
const fs = require('fs');
const path = require('path');
const { createClient } = require('redis');
const { Client } = require('pg');

const [A, B] = (process.env.SERVERS || 'http://127.0.0.1:3000,http://127.0.0.1:3001').split(',');
const TOKEN = process.env.TOKEN;
const N = +(process.env.N || 20);
const OUT = process.env.OUT || '.';
const WS = '20202020-1c25-4d02-bf25-6aeccf7ea419';
const DATA_KEY = `integration-tests:engine:core-entity:workspace:${WS}:data`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function gql(base, query, variables) {
  const res = await fetch(`${base}/metadata`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` },
    body: JSON.stringify({ query, variables }),
  });
  const body = await res.json();
  if (body.errors) throw new Error(JSON.stringify(body.errors).slice(0, 300));
  return body.data;
}
const rename = (base, name) =>
  gql(base, 'mutation($n: String!) { updateWorkspace(data: { displayName: $n }) { id } }', { n: name });

(async () => {
  const redis = createClient({ url: 'redis://127.0.0.1:6379' });
  await redis.connect();
  const pg = new Client({ host: '127.0.0.1', user: 'postgres', password: 'postgres', database: 'test' });
  await pg.connect();
  const runId = Date.now().toString(36);
  const results = [];
  for (let i = 1; i <= N; i++) {
    const a = `sync-${runId}-${i}-a`;
    const b = `sync-${runId}-${i}-b`;
    await redis.del(['cw:sync:arm', 'cw:sync:reached', 'cw:sync:release']);
    await rename(A, a);                       // 1
    await sleep(50);
    await redis.set('cw:sync:arm', '1');      // 2
    const read = gql(A, '{ currentWorkspace { id } }');
    const reached = await redis.blPop('cw:sync:reached', 10);
    await rename(B, b);                       // 3
    await redis.rPush('cw:sync:release', '1'); // 4
    await read;
    await sleep(100);
    const cached = await redis.get(DATA_KEY);  // 5
    const cachedName = cached ? JSON.parse(cached).displayName : null;
    const db = (await pg.query('select "displayName" from core.workspace where id = $1', [WS])).rows[0].displayName;
    const verdict = !reached ? 'NOT_REACHED' : cachedName === null ? 'ABSENT' : cachedName === db ? 'FRESH' : 'STALE';
    await gql(A, '{ currentWorkspace { id } }');  // 6
    await sleep(100);
    const after = await redis.get(DATA_KEY);
    const afterName = after ? JSON.parse(after).displayName : null;
    const refilled = afterName === db;
    results.push({ i, reached: !!reached, db, cached: cachedName, verdict, after: afterName, refilled });
    console.log(JSON.stringify(results[results.length - 1]));
  }
  const count = (v) => results.filter((r) => r.verdict === v).length;
  const summary = { fix: process.env.CW_FIX || 'none', n: N, stale: count('STALE'), fresh: count('FRESH'),
    absent: count('ABSENT'), not_reached: count('NOT_REACHED'),
    refilled_after: results.filter((r) => r.refilled).length };
  fs.writeFileSync(path.join(OUT, 'sync-results.json'), JSON.stringify({ summary, results }, null, 1));
  console.log('summary', JSON.stringify(summary));
  await redis.quit();
  await pg.end();
})().catch((e) => { console.error(e); process.exit(1); });
