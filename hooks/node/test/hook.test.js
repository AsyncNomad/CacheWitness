'use strict';
// Needs a Redis at CAWIT_SYNC_URL (run inside the isolated lab namespace).
const test = require('node:test');
const assert = require('node:assert');
const net = require('net');
const hook = require('..');

// A second plain connection playing the scenario runner.
function runner(url) {
  const m = /^redis:\/\/([^:/]+):(\d+)/.exec(url);
  const sock = net.connect(Number(m[2]), m[1]);
  let buf = Buffer.alloc(0);
  const waiting = [];
  sock.on('data', (d) => {
    buf = Buffer.concat([buf, d]);
    for (;;) {
      const r = hook._parse(buf, 0);
      if (!r) return;
      buf = buf.subarray(r[1]);
      waiting.shift()(r[0]);
    }
  });
  const cmd = (...a) => new Promise((res) => {
    waiting.push(res);
    sock.write(`*${a.length}\r\n` + a.map((x) => `$${Buffer.byteLength(x)}\r\n${x}\r\n`).join(''));
  });
  return { cmd, close: () => sock.destroy() };
}

test('parser handles bulk, array, null, integer, error', () => {
  assert.deepStrictEqual(hook._parse(Buffer.from('$3\r\nabc\r\n'), 0)[0], 'abc');
  assert.deepStrictEqual(hook._parse(Buffer.from('*2\r\n$1\r\na\r\n:5\r\n'), 0)[0], ['a', 5]);
  assert.strictEqual(hook._parse(Buffer.from('$-1\r\n'), 0)[0], null);
  assert.strictEqual(hook._parse(Buffer.from('*-1\r\n'), 0)[0], null);
  assert.ok(hook._parse(Buffer.from('-ERR x\r\n'), 0)[0] instanceof Error);
  assert.strictEqual(hook._parse(Buffer.from('$3\r\nab'), 0), null);
});

test('URL parsing matches the runner, and errors never show the password', () => {
  const p = hook._parseUrl;
  assert.deepStrictEqual(p('rediss://app:p%40ss@cache:6380/2'), { tls: true, host: 'cache', port: 6380, db: 2, user: 'app', password: 'p@ss' });
  assert.deepStrictEqual(p('redis://secret@h'), { tls: false, host: 'h', port: 6379, db: 0, user: null, password: 'secret' });
  assert.deepStrictEqual(p('redis://:only@[::1]:7000'), { tls: false, host: '::1', port: 7000, db: 0, user: null, password: 'only' });
  assert.throws(() => p('redis://u:hunter2@h/x'), (e) => !e.message.includes('hunter2') && e.message.includes('u:***@'));
  assert.throws(() => p('http://h'));
});

test('unarmed sync point returns at once', async () => {
  const t0 = Date.now();
  await hook.syncPoint('test:unarmed', 'k');
  assert.ok(Date.now() - t0 < 200);
});

test('armed sync point holds exactly one call until released', async () => {
  const r = runner(process.env.CAWIT_SYNC_URL);
  const token = `t${Date.now()}`;
  await r.cmd('SET', 'cawit:arm:test:point', `${token}|hold|*`, 'PX', '30000');
  let done1 = false;
  const p1 = hook.syncPoint('test:point', 'k1').then(() => { done1 = true; });
  const reached = await r.cmd('BLPOP', `cawit:reached:${token}`, '5');
  assert.ok(reached, 'hook signalled reached');
  assert.strictEqual(JSON.parse(reached[1]).key, 'k1');
  await hook.syncPoint('test:point', 'k2'); // arm consumed: passes through
  assert.strictEqual(done1, false, 'first call still held');
  await r.cmd('RPUSH', `cawit:release:${token}`, 'go');
  await p1;
  assert.strictEqual(done1, true);
  r.close();
});

test('key filter: a different key passes and leaves the arm in place', async () => {
  const r = runner(process.env.CAWIT_SYNC_URL);
  const token = `t${Date.now()}x`;
  await r.cmd('SET', 'cawit:arm:test:keyed', `${token}|hold|wanted`, 'PX', '30000');
  await hook.syncPoint('test:keyed', 'other');
  assert.strictEqual(await r.cmd('GET', 'cawit:arm:test:keyed'), `${token}|hold|wanted`);
  const p = hook.syncPoint('test:keyed', 'wanted');
  assert.ok(await r.cmd('BLPOP', `cawit:reached:${token}`, '5'));
  await r.cmd('RPUSH', `cawit:release:${token}`, 'go');
  await p;
  r.close();
});

test('a held call does not delay other sync points in the same process', async () => {
  const r = runner(process.env.CAWIT_SYNC_URL);
  const token = `t${Date.now()}h`;
  await r.cmd('SET', 'cawit:arm:test:held', `${token}|hold|*`, 'PX', '30000');
  const held = hook.syncPoint('test:held', 'k');
  assert.ok(await r.cmd('BLPOP', `cawit:reached:${token}`, '5'));
  await new Promise((res) => setTimeout(res, 100)); // let the hook enter its BLPOP
  const t0 = Date.now();
  await Promise.race([hook.syncPoint('test:other', 'k'), new Promise((res) => setTimeout(res, 1000))]);
  const waited = Date.now() - t0;
  await r.cmd('RPUSH', `cawit:release:${token}`, 'go');
  await held;
  r.close();
  assert.ok(waited < 500, `an unarmed sync point waited ${waited}ms behind a held one`);
});

test('action fail: signals reached, then throws once', async () => {
  const r = runner(process.env.CAWIT_SYNC_URL);
  const token = `t${Date.now()}f`;
  await r.cmd('SET', 'cawit:arm:test:fail', `${token}|fail|k`, 'PX', '30000');
  await assert.rejects(hook.syncPoint('test:fail', 'k'), hook.CawitInjectedError);
  const reached = await r.cmd('BLPOP', `cawit:reached:${token}`, '1');
  assert.strictEqual(JSON.parse(reached[1]).action, 'fail');
  await hook.syncPoint('test:fail', 'k'); // consumed: no second failure
  r.close();
});

test('production guard: NODE_ENV=production disables the hook unless allowed', () => {
  const { execFileSync } = require('child_process');
  const probe = (env) => execFileSync(process.execPath, ['-e', "process.stdout.write(String(require('..').enabled))"], {
    cwd: __dirname, env: { PATH: process.env.PATH, CAWIT_SYNC_URL: 'redis://127.0.0.1:1', ...env },
  }).toString();
  assert.strictEqual(probe({}), 'true');
  assert.strictEqual(probe({ NODE_ENV: 'production' }), 'false');
  assert.strictEqual(probe({ NODE_ENV: 'production', CAWIT_ALLOW_PRODUCTION: '1' }), 'true');
  assert.strictEqual(probe({ CAWIT_SYNC_URL: '' }), 'false');
});

test.after(() => hook.close());
