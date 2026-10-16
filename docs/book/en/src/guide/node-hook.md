# The Node Hook

`cawit-hook` is the Node implementation of the [hook protocol](../reference/hook-protocol.md). It has no dependencies and ships with TypeScript types.

```bash
npm install --save-dev cawit-hook
```

```ts
import * as cawit from "cawit-hook";          // ESM / TypeScript
// const cawit = require("cawit-hook");       // CommonJS
```

## API

### `syncPoint(point: string, key?: string): Promise<void>`

A named point where a scenario may hold or fail the call.

- `point`: the sync point name used by the scenario (`refill:after-read`, `invalidate:before`, `write:before-commit`, or your own).
- `key`: optional. The scenario can arm the point for one key only; calls with other keys pass through without touching the arm. Defaults to `*`.

Behaviour:

| State | What the call does |
|---|---|
| hook disabled (no `CAWIT_SYNC_URL`, or the production guard) | returns immediately, no I/O |
| enabled, point not armed (or armed for another key) | one Redis round trip, then returns |
| armed with `hold` | signals the runner, then waits until released (or until `CAWIT_SYNC_TIMEOUT_S`) |
| armed with `fail` | signals the runner, then throws `CawitInjectedError` (once) |

### `enabled: boolean`

Whether the hook is active in this process (read once at load).

### `CawitInjectedError`

The error thrown when a point armed with `fail` is taken.

### `close(): void`

Closes the hook's shared Redis connection (for tests and graceful shutdown).

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `CAWIT_SYNC_URL` | unset | `redis://host[:port][/db]` that the runner also uses. Unset means the hook is off |
| `CAWIT_SYNC_TIMEOUT_S` | `30` | how long a held call waits for release before going on (decimals allowed). The runner treats a hold that timed out as `NOT_REACHED` |
| `CAWIT_ALLOW_PRODUCTION` | unset | with `NODE_ENV=production`, the hook stays off unless this is `1` |

## Where to put the call

**Refill (stale-set).** Between the database read and the cache write:

```ts
const row = await repo.findOneBy({ id });
await cawit.syncPoint("refill:after-read", `article:${id}`);
await cache.set(`article:${id}`, row, ttl);
```

With a read-through helper such as `cache.wrap(key, loader)`, put the call at the end of the loader, after the database read:

```ts
return cache.wrap(`article:${id}`, async () => {
  const row = await repo.findOneBy({ id });
  await cawit.syncPoint("refill:after-read", `article:${id}`);
  return row;
});
```

**Invalidation (failed-invalidation).** Right before the call that removes the entry:

```ts
await cawit.syncPoint("invalidate:before", `article:${id}`);
await cache.del(`article:${id}`);
```

**Before commit (invalidate-before-commit).** As the last statement inside the transaction:

```ts
await db.transaction(async (trx) => {
  await trx("articles").update({ title }).where({ id });
  await cache.del(`article:${id}`);              // the bug under test
  await cawit.syncPoint("write:before-commit", `article:${id}`);
});
```

## Connections

The hook never uses your application's Redis connection. Non-blocking commands share one hook connection; each held call waits on a connection of its own, so a held request never delays other requests or other sync points in the same process. The hook connection is opened lazily, on the first call while enabled.

## Bundlers

If your build bundles dependencies (tsdown, esbuild, webpack, ncc), mark `cawit-hook` as external or rebuild after upgrading it. A copy of an older hook inside the build speaks an older protocol, never takes the arm, and every run reports `NOT_REACHED`. This happened in the [Directus case study](../case-studies/directus.md#what-we-learned).

## Production safety

- Without `CAWIT_SYNC_URL` the call returns before doing anything.
- With `NODE_ENV=production` the hook stays off even if `CAWIT_SYNC_URL` is set, unless `CAWIT_ALLOW_PRODUCTION=1`. Set that only in test environments that run with `NODE_ENV=production` (staging-like end-to-end tests).
- Even when enabled, a point that is not armed costs one Redis round trip per call. Do not enable the hook where that latency matters.

## Other languages

The protocol is a handful of Redis commands ([Hook Protocol](../reference/hook-protocol.md)). With an existing Redis client library, a hook for another language is a few dozen lines (the Node hook is about 150 because it carries its own RESP parser to stay dependency-free). Python is planned.
