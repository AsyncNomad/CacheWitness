# Case Study: Twenty CRM

[Twenty](https://github.com/twentyhq/twenty) is an open-source CRM (NestJS, TypeORM, PostgreSQL, Redis). We tested its core entity cache, which caches the workspace record that every authenticated request reads.

All runs were local, with the application lab isolated from the network. Pinned commit: `c3ba1401`. Lab files: [`lab/twenty/`](https://github.com/AsyncNomad/CacheWitness/tree/main/lab/twenty).

## The cache

- On a miss, `CoreEntityCacheService.recomputeFromProvider` reads the entity from PostgreSQL and writes it to Redis unconditionally (`MSET`).
- Invalidation deletes the entry after the write.
- Two in-process layers sit in front of Redis: a local cache that revalidates every 100 ms, and a memoizer that keeps a resolved value for 10 seconds.

## The hook

Four lines in the cache service: a comment, an import, and

```ts
await cawit.syncPoint('refill:after-read', `${cacheKeyName}:${entityId}`);
```

right after the provider's database read. The scenario arms it for `workspaceEntity:<workspace id>`, because one request refills several entities (user, workspace, API key) and only the workspace refill should be held.

## The scenario

[`lab/twenty/scenarios/stale-set.yaml`](https://github.com/AsyncNomad/CacheWitness/blob/main/lab/twenty/scenarios/stale-set.yaml): two servers, a GraphQL mutation that renames the workspace, a query that reads it, and `state` comparing the cached `displayName` with the database. GraphQL errors arrive with status 200, so both requests set `error_field: errors`.

## Results (20 runs each, every run reached the hook)

| Candidate fix | not_stale | refilled | Result |
|---|---|---|---|
| none | 0/20 | 0/20 | FAIL |
| version-compare `SET` (write only if the cached copy is not newer) | 0/20 | 0/20 | FAIL |
| lease (invalidation deletes the lease; write only if the lease survives) | 20/20 | 0/20 | FAIL |
| lease + re-read when the lease was lost | 20/20 | 20/20 | PASS |

## What we learned

- **Version-compare `SET` does nothing here.** After the newer write's `DEL`, the cache is empty; there is no newer copy to compare against, so the old value is written.
- **A lease alone looks correct but breaks the cache.** It prevents the stale write, but the request that lost its lease returns the value it read and the memoizer keeps serving it for 10 seconds while Redis stays empty. Only the `refilled` check catches this.
- **Natural frequency is low.** Without forcing the order, one person saving every 1 to 3 seconds produced no stale value in 477 saves (95% upper bound 0.8%). Concurrent writes produced brief stale reads (0.26% to 0.93% of writes) that lasted at most 43 ms, and no stale value outlived a burst of writes (0 of about 1,100). Only when the refill was slowed on purpose (50 ms) did values stay stale after bursts: 6.5% in one run and 0.17% in another, under identical conditions. A real race, but not one we would report as an urgent bug.
