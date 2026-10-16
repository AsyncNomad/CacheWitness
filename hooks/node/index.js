'use strict';
// CacheWitness sync hook for Node (protocol v2, docs/design/verify-design.md 3).
//
// syncPoint(point, key) is a no-op unless CAWIT_SYNC_URL is set, and also
// when NODE_ENV=production unless CAWIT_ALLOW_PRODUCTION=1 (a second guard
// against a stray CAWIT_SYNC_URL in production). When a scenario has armed
// `point`, the first call to reach it takes the arm (atomically, so only one
// call is taken) and signals cawit:reached:<token>. Armed with action `hold`,
// it then waits on cawit:release:<token>; with action `fail`, it throws
// CawitInjectedError, so a call placed before an operation (e.g. a cache
// invalidation) makes that operation fail.
//
// The hook never uses the app's cache connection. Non-blocking commands share
// one hook connection; each hold waits (BLPOP) on a connection of its own, so
// a held call never delays other sync points in the same process (a shared
// blocking connection made a second request in the same server wait for the
// first one's release).
const net = require('net');

const ENABLED = !!process.env.CAWIT_SYNC_URL &&
  (process.env.NODE_ENV !== 'production' || process.env.CAWIT_ALLOW_PRODUCTION === '1');
const TIMEOUT_S = Number(process.env.CAWIT_SYNC_TIMEOUT_S || 30);
let conn = null;

function parseUrl(u) {
  const m = /^redis:\/\/(?:[^@/]*@)?([^:/]+)(?::(\d+))?(?:\/(\d+))?/.exec(u);
  if (!m) throw new Error(`cawit-hook: bad CAWIT_SYNC_URL ${u}`);
  return { host: m[1], port: Number(m[2] || 6379), db: Number(m[3] || 0) };
}

// Minimal RESP2 client: requests are queued, replies resolve in order.
class Conn {
  constructor(url) {
    const { host, port, db } = parseUrl(url);
    this.queue = [];
    this.buf = Buffer.alloc(0);
    this.sock = net.connect(port, host);
    this.sock.setNoDelay(true);
    this.sock.on('data', (d) => this.onData(d));
    this.sock.on('error', (e) => this.failAll(e));
    this.sock.on('close', () => this.failAll(new Error('cawit-hook: connection closed')));
    if (db) this.cmd('SELECT', String(db));
  }
  cmd(...args) {
    let out = `*${args.length}\r\n`;
    for (const a of args) out += `$${Buffer.byteLength(a)}\r\n${a}\r\n`;
    return new Promise((resolve, reject) => {
      this.queue.push({ resolve, reject });
      this.sock.write(out);
    });
  }
  onData(d) {
    this.buf = Buffer.concat([this.buf, d]);
    for (;;) {
      const r = parse(this.buf, 0);
      if (!r) return;
      this.buf = this.buf.subarray(r[1]);
      const p = this.queue.shift();
      if (p) (r[0] instanceof Error ? p.reject : p.resolve)(r[0]);
    }
  }
  failAll(e) {
    for (const p of this.queue.splice(0)) p.reject(e);
    if (conn === this) conn = null;
  }
}

// Returns [value, nextOffset] or null if incomplete.
function parse(b, o) {
  const end = b.indexOf('\r\n', o);
  if (end < 0) return null;
  const line = b.toString('utf8', o + 1, end);
  const next = end + 2;
  switch (b[o]) {
    case 0x2b: return [line, next];                       // +
    case 0x2d: return [new Error(line), next];            // -
    case 0x3a: return [Number(line), next];               // :
    case 0x5f: return [null, next];                       // _ (RESP3 null)
    case 0x24: {                                          // $
      const n = Number(line);
      if (n < 0) return [null, next];
      if (b.length < next + n + 2) return null;
      return [b.toString('utf8', next, next + n), next + n + 2];
    }
    case 0x2a: {                                          // *
      const n = Number(line);
      if (n < 0) return [null, next];
      const items = [];
      let p = next;
      for (let i = 0; i < n; i++) {
        const r = parse(b, p);
        if (!r) return null;
        items.push(r[0]);
        p = r[1];
      }
      return [items, p];
    }
    default: return [new Error(`cawit-hook: unexpected reply ${String.fromCharCode(b[o])}`), next];
  }
}

function connection() {
  if (!conn) conn = new Conn(process.env.CAWIT_SYNC_URL);
  return conn;
}

// Take the arm only if it is for this key (or any key), in one step: a
// GETDEL-then-put-back would leave a gap in which the intended caller finds
// no arm. Arm value (protocol v2): <token>|<action>|<key or *>; returns
// {token, action} or nil.
const TAKE = `
local v = redis.call('GET', KEYS[1])
if not v then return false end
local s1 = string.find(v, '|', 1, true)
if not s1 then return false end
local s2 = string.find(v, '|', s1 + 1, true)
if not s2 then return false end
local want = string.sub(v, s2 + 1)
if want ~= '*' and want ~= ARGV[1] then return false end
redis.call('DEL', KEYS[1])
return {string.sub(v, 1, s1 - 1), string.sub(v, s1 + 1, s2 - 1)}`;

/** Thrown by syncPoint when a scenario armed the point with action `fail`. */
class CawitInjectedError extends Error {
  constructor(point, key) {
    super(`cawit: injected failure at ${point} (${key})`);
    this.name = 'CawitInjectedError';
  }
}

async function syncPoint(point, key = '*') {
  if (!ENABLED) return;
  const c = connection();
  const taken = await c.cmd('EVAL', TAKE, '1', `cawit:arm:${point}`, key);
  if (taken === null) return;
  const [token, action] = taken;
  await c.cmd('RPUSH', `cawit:reached:${token}`, JSON.stringify({ point, key, action, pid: process.pid }));
  if (action === 'fail') throw new CawitInjectedError(point, key);
  const waiter = new Conn(process.env.CAWIT_SYNC_URL);
  let released;
  try {
    released = await waiter.cmd('BLPOP', `cawit:release:${token}`, String(TIMEOUT_S));
  } finally {
    waiter.sock.destroy();
  }
  if (released === null) await c.cmd('RPUSH', `cawit:timeout:${token}`, String(process.pid));
}

function close() {
  if (conn) {
    conn.sock.destroy();
    conn = null;
  }
}

module.exports = { syncPoint, close, enabled: ENABLED, CawitInjectedError, _parse: parse };
