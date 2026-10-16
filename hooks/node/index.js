'use strict';
// CacheWitness sync hook for Node (protocol v1, docs/design/verify-design.md 3).
//
// syncPoint(point, key) is a no-op unless CAWIT_SYNC_URL is set. When a
// scenario has armed `point`, the first call to reach it takes the arm
// (GETDEL, so only one call is held), signals cawit:reached:<token>, and waits
// on cawit:release:<token>. The hook uses its own Redis connection: BLPOP
// blocks the connection, and blocking the app's cache connection would delay
// unrelated commands.
const net = require('net');

const ENABLED = !!process.env.CAWIT_SYNC_URL;
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

async function syncPoint(point, key = '*') {
  if (!ENABLED) return;
  const c = connection();
  const armKey = `cawit:arm:${point}`;
  const armed = await c.cmd('GETDEL', armKey);
  if (armed === null) return;
  const sep = armed.indexOf('|');
  const token = sep < 0 ? armed : armed.slice(0, sep);
  const want = sep < 0 ? '*' : armed.slice(sep + 1);
  if (want !== '*' && want !== key) {
    await c.cmd('SET', armKey, armed, 'PX', '30000'); // not ours: put it back
    return;
  }
  await c.cmd('RPUSH', `cawit:reached:${token}`, JSON.stringify({ point, key, pid: process.pid }));
  const released = await c.cmd('BLPOP', `cawit:release:${token}`, String(TIMEOUT_S));
  if (released === null) await c.cmd('RPUSH', `cawit:timeout:${token}`, String(process.pid));
}

function close() {
  if (conn) {
    conn.sock.destroy();
    conn = null;
  }
}

module.exports = { syncPoint, close, _parse: parse };
