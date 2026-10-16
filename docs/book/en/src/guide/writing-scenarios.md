# Writing a Scenario

This guide goes from "we suspect a stale-cache bug" to a scenario that fails before the fix and passes after it.

## 1. Pick the race

| Symptom | Likely race | Pattern |
|---|---|---|
| After an update, readers sometimes keep getting the old value until the TTL | a refill that started before the update finished after its invalidation | [`stale-set`](patterns.md#stale-set) |
| An update returned an error (or timed out) but the value changed, and the cache kept the old one | the invalidation failed after the commit | [`failed-invalidation`](patterns.md#failed-invalidation) |
| Readers get the old value right after an update, even with no concurrent readers before it | the cache is invalidated inside the transaction, before the commit | [`invalidate-before-commit`](patterns.md#invalidate-before-commit) |

## 2. Find where the hook goes

Each pattern needs a sync point at one place in your code:

| Pattern | Point (default name) | Where |
|---|---|---|
| `stale-set` | `refill:after-read` | in the miss path, after the database read and before the cache write |
| `failed-invalidation` | `invalidate:before` | right before the cache invalidation in the write path |
| `invalidate-before-commit` | `write:before-commit` | at the end of the write transaction, before the commit |

Search your code for the cache write (`set`, `mset`, `setex`, a cache library's `set` or `wrap`) and for the invalidation (`del`, `unlink`, `clear`, a tag or namespace purge). Frameworks often concentrate these in one service or middleware; in the case studies the hook went into one cache service (Twenty) and one response middleware (Directus).

Choose a **key** that lets the scenario hold only the refill it cares about. A single read request often refills several cache entries; if the point is armed for any key, an unrelated refill may be taken first. Use something the scenario can write literally, such as the request path or `<entity>:<id>`.

## 3. Requests

Write one request template for the write and one for the read. The write must use `${value}` (or another variable named by the pattern's `value_var`) for the value it writes, so that each run writes values unique to that run.

```yaml
requests:
  update:
    method: PATCH
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.API_TOKEN}" }
    json: { title: "${value}" }
  read:
    method: GET
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.API_TOKEN}" }
```

For GraphQL, add `error_field: errors`: GraphQL reports errors with status 200, and without this a failed mutation would look like a success.

## 4. State: what to compare

`db_query` returns the committed value, as text, in the first column of the first row.

Then choose how to read the cache side:

- **`cache_key`** when you know the Redis key and the value is plain text or JSON. Add `cache_field` to compare one top-level JSON field.
- **`app_view`** when keys are hashed, values are compressed or serialized, or what matters is the response. It sends a request through the application and takes a JSON pointer from the body. Add `hit_header` if your application reports cache hits in a header; the `refilled` check then also requires a hit.

## 5. One instance or two

A scenario can send the competing write through a second instance (`writer: b`) or through the same one (`writer: a`). Both work: each held call waits on its own Redis connection, so it does not block other requests in the same process. Two instances are closer to production and also exercise cross-instance invalidation.

## 6. Run, read, iterate

```bash
cawit verify scenario.yaml --show-steps   # check the expansion
cawit verify scenario.yaml --runs 3       # quick try
cawit verify scenario.yaml                # the configured number of runs
```

- **All `FAIL` before the fix:** good, the race is reproduced.
- **`NOT_REACHED`:** the hook was not reached, or the hold timed out. See [Troubleshooting](troubleshooting.md#not_reached).
- **`PASS` before any fix:** your code may already be safe against this order, or the scenario does not exercise the path you think. Check `--show-steps`, and confirm with the application's logs that the read actually missed.

## 7. Keep it

Commit the scenario next to your tests and run it in CI ([Running in CI](ci.md)). In the case studies, 20 runs of a pattern took 15 to 30 seconds.
