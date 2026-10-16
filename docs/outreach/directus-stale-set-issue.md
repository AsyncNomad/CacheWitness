# Draft: Directus issue — stale-set race in the response cache

> Status: DRAFT, not submitted. Audience: Directus maintainers. Submission is
> the project maintainer's decision. Before submitting: check Directus'
> issue template and CLA (`cla.md`), re-run on the latest release, and decide
> whether to attach a PR.

---

**Title:** Response cache can keep a stale item until `CACHE_TTL` when a read refills it concurrently with an update (stale-set race)

### Describe the bug

With `CACHE_ENABLED=true`, `CACHE_STORE=redis` and `CACHE_AUTO_PURGE=true`, a GET that misses the cache and is still building its response while another request updates the same item can write the **pre-update** response into the cache **after** the update's purge. The stale response is then served as a cache hit until the next purge or until `CACHE_TTL` expires (default 5 minutes).

The order:

1. GET `/items/articles/1` misses (`middleware/cache.ts`) and the controller reads the row (old value).
2. PATCH `/items/articles/1` commits and then purges the cache (`ItemsService.updateMany` → `cache.clear()` after the transaction).
3. The GET reaches `middleware/respond.ts` and calls `setCacheValue(...)` with the old payload.
4. Every later GET is a HIT with the old value; the database has the new one.

Purging after the commit (as Directus does) is correct and prevents the "invalidate before commit" variant; it does not prevent this one, because the refill that started before the purge finishes after it.

### To reproduce

Directus v12.5.0 (`4f77933b`), PostgreSQL 16, Redis 7.2. Reproduced with two API instances sharing Redis and with a single instance.

Deterministic reproduction: a test-only patch adds a pause point in `respond.ts` right before `setCacheValue`. A scenario holds one GET at that point, sends the PATCH (through the other instance, or the same one), then releases the GET:

- unpatched code: stale in **20 of 20** runs (two instances) and **10 of 10** (one instance); the next GET returns the old title with `X-Cache-Status: HIT`
- the pause point only changes timing; without it the same order happens on its own when saves come in quick succession (next section)

Scenario, patch, and scripts: <link to CacheWitness repo, lab/directus/>. The patch is offered under MSCL-1.0, like the code it modifies.

### How likely is it without forcing the order?

Measured locally without any pause point or patch (two instances, one item, readers on both instances, one writer). After each write, or each group of back-to-back writes, we waited 300 ms and read through an instance; a value different from the database counts as stale. Two independent runs per condition:

| Writes | Readers (per instance) | Run 1 | Run 2 |
|---|---|---|---|
| spaced 0.5 s apart | 2 (~75 req/s total) or 8 (~700-1,000 req/s) | 0/300, 0/300 | 0/300, 0/300 |
| 2 back to back | 2 | 10/200 (5.0%) | 4/200 (2.0%) |
| 2 back to back | 8 | 1/200 (0.5%) | 0/200 |
| 5 back to back | 2 | 12/200 (6.0%) | 5/200 (2.5%) |
| 5 back to back | 8 | 2/200 (1.0%) | 1/200 (0.5%) |

- With writes spaced out we never saw it (0 of 1,200; 95% upper bound about 0.25%): by the time the next write lands, the cache is warm and no refill is in flight.
- With several saves in quick succession (an editor saving twice, a script updating an item a few times), the previous value stayed cached in 0-6% of the groups, always served as a HIT, until the next write or `CACHE_TTL`.
- Rates varied up to 2.5x between identical runs, so treat them as a range. Local test setup, not production traffic.

### Possible fix

A purge generation: increment a counter in Redis before each data-cache purge; a GET that missed records the counter before reading the database, and after writing the cache it re-checks the counter and deletes its entry if a purge happened in between. Order matters: writer `commit → INCR → purge`, reader `read counter → read DB → SET → re-check`. In the same deterministic scenario this passed 20 of 20 runs and the cache still refilled. Cost: one GET on a miss and one GET after each cache write (no extra work on hits).

Alternatives we checked and why they are not enough:

- **Delayed second purge** (purge again N ms later): passes when the GET is fast, fails 20 of 20 when the GET is slower than N.
- **Shorter `CACHE_TTL`**: bounds how long the stale value lives; does not prevent it.

Happy to open a PR with the generation check and a regression test if this approach is acceptable.

### Disclosure

Found and verified with CacheWitness (<link>), a testing tool I am building for cache consistency races; the reproduction above does not require it beyond the scenario runner.
