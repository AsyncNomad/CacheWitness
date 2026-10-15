// Minimal race amplifier for the Twenty lab (a stand-in for PLAN 9.9).
//
// Relays Redis traffic from LISTEN to UPSTREAM byte for byte, but holds back
// any SET/MSET that writes a key matching AMP_KEY_REGEX for AMP_DELAY_MS
// before forwarding it. Commands on one connection stay in order, so a held
// command also holds the ones behind it (the head-of-line blocking described
// in PLAN 9.9). Replies are piped back untouched.
//
// Env: LISTEN (6380)  UPSTREAM (6379)  AMP_DELAY_MS (5)  AMP_JITTER (0; 1 = uniform 0..ms)
//      AMP_KEY_REGEX (core-entity:workspace:[^:]+:data$)
const net = require('net');

const LISTEN = +(process.env.LISTEN || 6380);
const UPSTREAM = +(process.env.UPSTREAM || 6379);
const DELAY = +(process.env.AMP_DELAY_MS || 5);
const JITTER = process.env.AMP_JITTER === '1';
const KEY_RX = new RegExp(process.env.AMP_KEY_REGEX || 'core-entity:workspace:[^:]+:data$');
// queuedMax: most commands waiting at once on one connection; waitMaxMs and
// waitTotalMs: time commands spent waiting behind held ones (head-of-line).
const stats = { conns: 0, commands: 0, held: 0, queuedMax: 0, waitMaxMs: 0, waitTotalMs: 0, waited: 0 };

// Parse one RESP request (array of bulk strings) from buf at off.
// Returns [args, nextOffset] or null if incomplete.
function parseRequest(buf, off) {
  if (off >= buf.length) return null;
  if (buf[off] !== 0x2a) {
    // inline command: up to CRLF
    const end = buf.indexOf('\r\n', off);
    if (end < 0) return null;
    return [buf.toString('utf8', off, end).split(' '), end + 2];
  }
  let end = buf.indexOf('\r\n', off);
  if (end < 0) return null;
  const n = parseInt(buf.toString('latin1', off + 1, end), 10);
  let p = end + 2;
  const args = [];
  for (let i = 0; i < n; i++) {
    if (p >= buf.length) return null;
    end = buf.indexOf('\r\n', p);
    if (end < 0) return null;
    const len = parseInt(buf.toString('latin1', p + 1, end), 10);
    const start = end + 2;
    if (start + len + 2 > buf.length) return null;
    args.push(buf.toString('latin1', start, start + len));
    p = start + len + 2;
  }
  return [args, p];
}

function shouldHold(args) {
  const cmd = (args[0] || '').toUpperCase();
  if (cmd === 'SET' || cmd === 'SETEX' || cmd === 'PSETEX') return KEY_RX.test(args[1] || '');
  if (cmd === 'MSET') return args.slice(1).some((a, i) => i % 2 === 0 && KEY_RX.test(a));
  return false;
}

net.createServer((client) => {
  stats.conns++;
  const upstream = net.connect(UPSTREAM, '127.0.0.1');
  client.setNoDelay(true);
  upstream.setNoDelay(true);
  upstream.pipe(client);
  let pending = Buffer.alloc(0);
  let chain = Promise.resolve();
  let queued = 0;
  client.on('data', (chunk) => {
    pending = Buffer.concat([pending, chunk]);
    let off = 0;
    for (;;) {
      const r = parseRequest(pending, off);
      if (!r) break;
      const [args, next] = r;
      const bytes = pending.subarray(off, next);
      stats.commands++;
      const hold = shouldHold(args);
      if (hold) stats.held++;
      const arrived = Date.now();
      queued++;
      if (queued > stats.queuedMax) stats.queuedMax = queued;
      chain = chain.then(async () => {
        // Time spent behind earlier held commands, before this one's own hold.
        const waited = Date.now() - arrived;
        if (waited > 0) {
          stats.waited++;
          stats.waitTotalMs += waited;
          if (waited > stats.waitMaxMs) stats.waitMaxMs = waited;
        }
        if (hold) await new Promise((res) => setTimeout(res, JITTER ? Math.random() * DELAY : DELAY));
        upstream.write(bytes);
        queued--;
      });
      off = next;
    }
    pending = pending.subarray(off);
  });
  const close = () => { client.destroy(); upstream.destroy(); };
  client.on('error', close);
  upstream.on('error', close);
  client.on('close', close);
  upstream.on('close', close);
}).listen(LISTEN, '127.0.0.1', () => {
  console.log(`amp-proxy :${LISTEN} -> :${UPSTREAM}, holding SET/MSET of ${KEY_RX} for ${DELAY}ms`);
});

setInterval(() => console.log('stats', JSON.stringify(stats)), 5000).unref();
// Final counts for the manipulation check (docker stop sends SIGTERM).
process.on('SIGTERM', () => { console.log('stats', JSON.stringify(stats)); process.exit(0); });
