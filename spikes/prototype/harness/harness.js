// Injects known cache-consistency bugs (and correct patterns) through the
// CacheWitness proxy, with background traffic, and records ground truth.
const { Pool } = require('pg');
const { createClient } = require('redis');
const fs = require('fs');

const PROXY = process.env.PROXY || 'redis://127.0.0.1:6380';
const N = +(process.env.N || 40);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const pool = new Pool({ host: '127.0.0.1', user: 'postgres', database: 'app', max: 40 });

async function client(opts = {}) { const c = createClient({ url: PROXY, ...opts }); await c.connect(); return c; }

const row = async (id, db = pool) => (await db.query('select id,name,version from users where id=$1', [id])).rows[0];
const enc = (r) => JSON.stringify({ id: r.id, name: r.name, version: r.version });
const bumpRow = (id, db = pool) => db.query("update users set name=md5(random()::text), version=version+1 where id=$1", [id]);

let R, W, BAD, BG; // reader, writer, no-DEL-permission writer, background
const truth = {};

const scenarios = {
  // ---- correct patterns: must NOT alarm
  async correct_cache_aside(id) {
    await R.set(`user:${id}`, enc(await row(id)), { EX: 3600 });
    const reads = (async () => { for (let i = 0; i < 10; i++) { await R.get(`user:${id}`); await sleep(3); } })();
    await bumpRow(id);
    await sleep(20);                         // gap between commit and DEL: transiently stale, by design
    await W.del(`user:${id}`);
    await reads;
    const v = await R.get(`user:${id}`);     // reader refills from DB after commit -> fresh
    if (!v) await R.set(`user:${id}`, enc(await row(id)), { EX: 3600 });
    return null;
  },
  async correct_write_through(id) {
    await R.set(`user:${id}`, enc(await row(id)));
    await bumpRow(id);
    await W.set(`user:${id}`, enc(await row(id)));
    return null;
  },
  // ---- bugs
  async missing_invalidation(id) {
    await R.set(`user:${id}`, enc(await row(id)), { EX: 3600 });
    await bumpRow(id);                       // forgot to DEL
    return 'MISSING_INVALIDATION';
  },
  async stale_set_race(id) {
    await W.del(`user:${id}`);
    const reader = (async () => {
      if (!(await R.get(`user:${id}`))) {
        const old = await row(id);           // reads OLD row
        await sleep(60);                     // slow request (GC pause, network...)
        await R.set(`user:${id}`, enc(old), { EX: 3600 });
      }
    })();
    await sleep(20);
    await bumpRow(id);
    await W.del(`user:${id}`);               // correct invalidation... but reader writes back old value
    await reader;
    return 'STALE_SET_RACE';
  },
  async invalidate_before_commit(id) {
    const c = await pool.connect();
    try {
      await c.query('begin');
      await bumpRow(id, c);
      await W.del(`user:${id}`);             // BUG: invalidates inside the transaction
      const reader = (async () => {
        await sleep(10);
        if (!(await R.get(`user:${id}`))) await R.set(`user:${id}`, enc(await row(id)), { EX: 3600 }); // sees old row
      })();
      await sleep(50);
      await c.query('commit');
      await reader;
    } finally { c.release(); }
    return 'INVALIDATE_BEFORE_COMMIT';
  },
  async failed_delete(id) {
    await R.set(`user:${id}`, enc(await row(id)), { EX: 3600 });
    await bumpRow(id);
    try { await BAD.del(`user:${id}`); } catch (e) { /* swallowed, not retried */ }
    return 'INVALIDATION_FAILED';
  },
  async replica_lag(id) {
    const replicaSnapshot = await row(id);  // what a lagging replica would still return
    await W.del(`user:${id}`);
    await bumpRow(id);
    await W.del(`user:${id}`);
    await sleep(5);
    if (!(await R.get(`user:${id}`))) await R.set(`user:${id}`, enc(replicaSnapshot), { EX: 3600 });
    return 'STALE_READ_AFTER_INVALIDATION';
  },
  async ttl_by_design(id) {
    await R.set(`user:${id}`, enc(await row(id)), { EX: 3 }); // short TTL, no invalidation on purpose
    await bumpRow(id);
    return 'BOUNDED_BY_TTL';
  },
};

async function background(stop) {
  // write-through traffic on ids 4001..5000 (never stale) + reads, to test false positives under load
  let ops = 0;
  while (Date.now() < stop.t) {
    const id = 4001 + Math.floor(Math.random() * 1000);
    if (Math.random() < 0.3) { await bumpRow(id); await BG.set(`user:${id}`, enc(await row(id))); }
    else await BG.get(`user:${id}`);
    ops++;
  }
  return ops;
}

(async () => {
  R = await client(); W = await client(); BG = await client();
  BAD = await client({ username: 'noinv', password: 'x' });
  // reset cache for test ids
  const direct = createClient({ url: 'redis://127.0.0.1:6379' }); await direct.connect(); await direct.flushAll(); await direct.quit();
  for (let id = 4001; id <= 5000; id++) await BG.set(`user:${id}`, enc(await row(id)));

  const bgStop = { t: Date.now() + 600000 };
  const bgWorkers = Array.from({ length: 4 }, () => background(bgStop));
  const start = Date.now();
  let id = 1;
  const jobs = [];
  for (const [name, fn] of Object.entries(scenarios)) {
    const reps = name.startsWith('correct') ? N * 2 : N;
    for (let i = 0; i < reps; i++) {
      const myId = id++;
      jobs.push({ name, fn, id: myId });
    }
  }
  // shuffle and run with bounded concurrency
  jobs.sort(() => Math.random() - 0.5);
  let next = 0;
  await Promise.all(Array.from({ length: 16 }, async () => {
    while (next < jobs.length) {
      const j = jobs[next++];
      const label = await j.fn(j.id);
      truth[`user:${j.id}`] = { scenario: j.name, expected: label };
    }
  }));
  const dur = (Date.now() - start) / 1000;
  await sleep(1500);            // let checks fire
  bgStop.t = 0;
  const bgOps = (await Promise.all(bgWorkers)).reduce((a, b) => a + b, 0);
  fs.writeFileSync('truth.json', JSON.stringify(truth, null, 1));
  console.log(JSON.stringify({ cases: jobs.length, scenario_seconds: dur, background_ops: bgOps }));
  await Promise.all([R.quit(), W.quit(), BG.quit(), BAD.quit()]);
  await pool.end();
})().catch((e) => { console.error(e); process.exit(1); });
