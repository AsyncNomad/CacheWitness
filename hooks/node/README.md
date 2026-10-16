# cawit-hook

Sync hook for [CacheWitness](https://github.com/AsyncNomad/CacheWitness): put one call in your cache refill path so a test can force a cache race in the same order on every run. No dependencies.

```ts
import * as cawit from 'cawit-hook';

const row = await db.findOne(id);
await cawit.syncPoint('refill:after-read', key); // no-op unless a test armed it
await cache.set(key, serialize(row));
```

- **Off by default.** Without `CAWIT_SYNC_URL` the call returns at once with no I/O. With `NODE_ENV=production` it also stays off unless `CAWIT_ALLOW_PRODUCTION=1`. `cawit.enabled` tells whether it is on.
- **Hold.** When a scenario run by `cawit verify` arms the point (optionally for one key), the first call to reach it signals the runner and waits until released. Each held call waits on its own Redis connection, so other sync points in the process are not delayed.
- **Fail.** Armed with action `fail`, the call throws `CawitInjectedError` once. Place it right before an operation, such as a cache invalidation, to test what happens when that operation fails.

Environment: `CAWIT_SYNC_URL` (`redis://host:port[/db]`, a Redis the test runner also uses), `CAWIT_SYNC_TIMEOUT_S` (how long a held call waits before going on, default 30), `CAWIT_ALLOW_PRODUCTION`.

**Bundlers:** if your build bundles dependencies, mark `cawit-hook` as external, or rebuild after upgrading it. A copy of an older hook baked into the build speaks an older protocol and every run reports `NOT_REACHED`.

For test environments only. Licensed under Apache-2.0.
