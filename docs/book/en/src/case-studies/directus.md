# Case Study: Directus

[Directus](https://github.com/directus/directus) is a data platform with a REST and GraphQL API (Express, knex). With `CACHE_ENABLED=true`, `CACHE_STORE=redis`, and `CACHE_AUTO_PURGE=true` it caches whole GET responses in Redis and purges the cache namespace after every write. Its cache differs from Twenty's in every layer, which makes it a good second target.

| | Twenty | Directus |
|---|---|---|
| Framework / ORM | NestJS / TypeORM | Express / knex |
| Cached unit | an entity | an HTTP response body |
| Key | `…:workspace:<id>:data` | hash of version, user, path, query |
| Value | JSON | compressed |
| Invalidation | delete one key | purge the whole namespace |
| In-process cache | local cache + memoizer | none |

Version: v12.5.0 (`4f77933b`), two API instances sharing Redis. All runs local and network-isolated. Lab files: [`lab/directus/`](https://github.com/AsyncNomad/CacheWitness/tree/main/lab/directus).

## Hooks

| Point | Where | Lines |
|---|---|---|
| `refill:after-read` | `middleware/respond.ts`, before `setCacheValue`, key `req.baseUrl + req.path` | 3 |
| `invalidate:before` | the data-cache purge in `cache.ts` | 2 |
| `write:before-commit` | end of the transaction in `ItemsService.updateMany` | 2 |

Because keys are hashed and values compressed, the scenarios judge through the application: `state.app_view` reads `/data/title` from `GET /items/articles/1`, and `X-Cache-Status` (`CACHE_STATUS_HEADER`) tells a hit from a database read.

## Results (20 runs each, every run reached its sync point)

### stale-set

| Fix | Result |
|---|---|
| none | FAIL 20/20: the old title served as a cache hit |
| delayed second purge (500 ms), reader held 1 s | FAIL 20/20 |
| generation check (bump a counter before purging; drop a refill that started under an older one) | PASS 20/20 |

Control: with the reader released immediately, the delayed second purge **passed 10/10**. It only fails when the reader is slower than the delay, which is exactly the case ordinary tests do not produce. With one instance instead of two: none FAIL 10/10, generation check PASS 10/10.

### failed-invalidation

| Fix | Strict | Within 3 s |
|---|---|---|
| none (TTL 5 min) | FAIL 20/20 | FAIL 20/20 |
| short TTL (2 s) | FAIL 20/20 | PASS 20/20 |
| retry the purge | PASS 20/20 | PASS 20/20 |

Without a fix, the write returns 500 after its commit: the client sees a failure, the database has the new value, and the cache keeps the old one.

### invalidate-before-commit

| Configuration (`hold_ms: 1000`) | Result |
|---|---|
| upstream Directus (purge after commit) | PASS 20/20 |
| purge moved into the transaction (lab bug) | FAIL 20/20 |
| that, plus a delayed second purge | FAIL 20/20 |

Upstream passes: the pattern does not flag correct code.

## How often without forcing?

Two instances, readers on both, one writer, no hook and no fix; after each write (or group of back-to-back writes) a check read 300 ms later. Two independent runs per condition:

| Writes | Readers per instance | Run 1 | Run 2 |
|---|---|---|---|
| 0.5 s apart | 2 or 8 | 0/300, 0/300 | 0/300, 0/300 |
| 2 back to back | 2 | 5.0% | 2.0% |
| 2 back to back | 8 | 0.5% | 0% |
| 5 back to back | 2 | 6.0% | 2.5% |
| 5 back to back | 8 | 1.0% | 0.5% |

The race needs a refill in flight when a write lands, so it appears only when saves come in quick succession; then the previous value can stay cached, as a hit, until the next write or `CACHE_TTL`.

## What we learned

- **Bundlers can freeze an old hook.** Directus' build copied the hook into its output. After the protocol changed, every run was `NOT_REACHED`; Redis `MONITOR` showed the old script. The fix was to mark `cawit-hook` external.
- **Router-relative paths.** The first runs were `NOT_REACHED` because `req.path` inside a mounted router lacks the mount prefix. The manipulation check turned a silent misconfiguration into a visible one.
- **A shared wait connection broke single-instance scenarios.** An early hook waited on its shared connection, so a second request in the same process queued behind the held one until the hold timed out, and the run passed with the order broken. Holds now wait on their own connections, and the runner discards runs whose hold timed out.
- **Timers need pinned time.** Without `hold_ms`, the delayed-purge fix flipped once in 20 runs, on the cold first run. See [Race Patterns](../guide/patterns.md#timing-parameters).
