// "Natural" test: textbook cache-aside (DEL after commit) under heavy concurrency
// on hot keys. No bug is injected; any stale entries arise from real races.
// Oracle: after quiescence, compare every hot key in Redis with the DB.
const { Pool } = require('pg');
const { createClient } = require('redis');
const fs = require('fs');
const PROXY = process.env.PROXY || 'redis://127.0.0.1:6380';
const SECONDS = +(process.env.SECONDS || 20), KEYS = +(process.env.KEYS || 200), WORKERS = +(process.env.WORKERS || 32);
const pool = new Pool({ host: '127.0.0.1', user: 'postgres', database: 'app', max: WORKERS + 4 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const enc = (r) => JSON.stringify({ id: r.id, name: r.name, version: r.version });

(async () => {
  const clients = await Promise.all(Array.from({ length: 8 }, async () => { const c = createClient({ url: PROXY }); await c.connect(); return c; }));
  const base = 2001; // ids 2001..2000+KEYS
  const stop = Date.now() + SECONDS * 1000;
  let reads = 0, writes = 0, fills = 0;
  await Promise.all(Array.from({ length: WORKERS }, async (_, w) => {
    const c = clients[w % clients.length];
    while (Date.now() < stop) {
      const id = base + Math.floor(Math.random() * KEYS);
      const key = `user:${id}`;
      if (Math.random() < 0.7) {
        reads++;
        if (!(await c.get(key))) {
          const r = (await pool.query('select id,name,version from users where id=$1', [id])).rows[0];
          await sleep(Math.random() * 5);         // realistic app-side delay between DB read and cache fill
          await c.set(key, enc(r), { EX: 3600 });
          fills++;
        }
      } else {
        writes++;
        await pool.query("update users set name=md5(random()::text), version=version+1 where id=$1", [id]);
        await c.del(key);                          // correct order: after commit
      }
    }
  }));
  await sleep(2000);
  const direct = createClient({ url: 'redis://127.0.0.1:6379' }); await direct.connect();
  const stale = [];
  for (let id = base; id < base + KEYS; id++) {
    const v = await direct.get(`user:${id}`);
    if (!v) continue;
    const r = (await pool.query('select version from users where id=$1', [id])).rows[0];
    const cv = JSON.parse(v).version;
    if (cv !== r.version) stale.push({ key: `user:${id}`, cached_version: cv, db_version: r.version });
  }
  fs.writeFileSync('natural_oracle.json', JSON.stringify(stale, null, 1));
  console.log(JSON.stringify({ reads, writes, fills, ops_per_s: Math.round((reads + writes) / SECONDS), stale_at_end: stale.length }));
  await direct.quit(); await Promise.all(clients.map((c) => c.quit())); await pool.end();
})();
