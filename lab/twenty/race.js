// Phase 1 race experiment against a running Twenty server.
//
// One admin renames the workspace every WRITE_INTERVAL_MS (displayName
// "race-<n>", so every cached value names the version it came from) while
// READERS concurrent requests authenticate against the same workspace. Every
// authenticated request reads the workspace entity cache in the JWT strategy,
// so readers are what refill the cache after each invalidation.
//
// Env:
//   SERVERS (http://127.0.0.1:3000)  comma-separated base URLs; requests rotate over them
//   TOKEN                            admin access token for the workspace
//   READERS (50)  SECONDS (30)  WRITE_INTERVAL_MS (100)  OUT (.)
const fs = require('fs');
const path = require('path');

const cfg = {
  servers: (process.env.SERVERS || 'http://127.0.0.1:3000').split(','),
  token: process.env.TOKEN,
  readers: +(process.env.READERS || 50),
  seconds: +(process.env.SECONDS || 30),
  writeIntervalMs: +(process.env.WRITE_INTERVAL_MS || 100),
  out: process.env.OUT || '.',
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let rr = 0;
async function gql(query, variables) {
  const base = cfg.servers[rr++ % cfg.servers.length];
  const res = await fetch(`${base}/metadata`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', authorization: `Bearer ${cfg.token}` },
    body: JSON.stringify({ query, variables }),
  });
  const body = await res.json();
  if (body.errors) throw new Error(JSON.stringify(body.errors).slice(0, 300));
  return body.data;
}

(async () => {
  const stop = Date.now() + cfg.seconds * 1000;
  const writes = [];
  const reads = { ok: 0, failed: 0, errors: {} };

  const reader = async () => {
    while (Date.now() < stop) {
      try {
        await gql('{ currentWorkspace { id } }');
        reads.ok++;
      } catch (e) {
        reads.failed++;
        const k = String(e.message).slice(0, 120);
        reads.errors[k] = (reads.errors[k] || 0) + 1;
      }
    }
  };

  const writer = async () => {
    let n = 0;
    while (Date.now() < stop) {
      n++;
      const s = Date.now();
      try {
        await gql('mutation($n: String!) { updateWorkspace(data: { displayName: $n }) { id } }', { n: `race-${n}` });
        writes.push({ n, s, e: Date.now(), ok: true });
      } catch (e) {
        writes.push({ n, s, e: Date.now(), ok: false, err: String(e.message).slice(0, 200) });
      }
      await sleep(cfg.writeIntervalMs);
    }
  };

  await Promise.all([writer(), ...Array.from({ length: cfg.readers }, reader)]);
  fs.writeFileSync(path.join(cfg.out, 'writes.json'), JSON.stringify(writes));
  const summary = {
    cfg: { ...cfg, token: undefined },
    writes: writes.length, writes_ok: writes.filter((w) => w.ok).length,
    last_written: writes.filter((w) => w.ok).map((w) => `race-${w.n}`).pop(),
    reads,
  };
  fs.writeFileSync(path.join(cfg.out, 'race.json'), JSON.stringify(summary, null, 1));
  console.log(JSON.stringify(summary));
})().catch((e) => { console.error(e); process.exit(1); });
