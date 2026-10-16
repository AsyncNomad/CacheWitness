# Troubleshooting

## NOT_REACHED

```text
run   1  NOT_REACHED
          sync point `refill:after-read` not reached within 10000ms
```

The armed call never arrived, so the forced order did not happen. Check, in order:

1. **The hook is enabled in the application.** `CAWIT_SYNC_URL` must be set in the application's environment (not only the runner's), and with `NODE_ENV=production` also `CAWIT_ALLOW_PRODUCTION=1`. Log `require("cawit-hook").enabled` at startup to be sure.
2. **Same Redis.** The application's `CAWIT_SYNC_URL` and the scenario's `redis` must reach the same Redis server and the same database number.
3. **The request takes the path you hooked.** A read that hits the cache never reaches a refill hook: the pattern's first write invalidates the entry, but a cache in front of Redis (an in-process LRU, an HTTP cache) can still answer. Check the application log for a database read.
4. **The key matches.** If the scenario arms with `key: X`, the hook must be called with exactly `X`. Frameworks often see a different path than the client sends: in Express, `req.path` inside a mounted router lacks the mount prefix (use `req.baseUrl + req.path`).
5. **The build contains the current hook.** A bundler can bake an older copy of `cawit-hook` into the build. Watch Redis while a run executes:

   ```bash
   redis-cli monitor | grep -E 'cawit:|EVAL'
   ```

   You should see the runner's `SET cawit:arm:<point>`, then the hook's `EVAL`, then `RPUSH cawit:reached:<token>`. If the hook's `EVAL` script looks different from the current one, rebuild.

## "the hold at … timed out before release"

The held call stopped waiting before the scenario released it, so the order was broken and the run was discarded. Either the scenario waits longer between `reached` and `release` than the hook's `CAWIT_SYNC_TIMEOUT_S` (default 30 s), or a step in between (an HTTP request) took that long. Raise the timeout in the application's environment, or find the slow step.

## ERROR

```text
run   1  ERROR
          step 6 (http): HTTP 500 from http://127.0.0.1:3001/items/articles/1: …
```

A request returned an error status (or a GraphQL `errors` field with `error_field`), a check could not read its value, or a step's configuration is wrong. The note names the step. If the request is expected to fail (fault injection), set `allow_error: true` on that `http` step; the `failed-invalidation` pattern does this for you.

## Every run passes before any fix

- The race may really be closed for this order (for example, the code invalidates after commit and the pattern is `invalidate-before-commit`).
- Or the scenario does not exercise the path: run with `--show-steps`, check that the read actually missed and refilled (application logs), and that `state` compares the right value. With `app_view`, a `hit_header` makes the `refilled` check prove the value came from the cache.

## Results differ between runs

Within the order and the waits a scenario controls, runs are deterministic. If results vary:

- the code under test likely relies on a timer; pin the timing with `hold_ms` or use `within_ms` ([Race Patterns](patterns.md#timing-parameters));
- the first run after start-up is slower (cold caches, JIT, connection pools);
- the scenario shares the database or cache with something else running at the same time.

## "unknown variable ${…}" or "environment variable … is not set"

Templates fail loudly instead of sending a literal `${...}`. Define the variable in the step's `vars`, use `${run}`/`${run_index}`, or export the environment variable for the `cawit` process.

## "unknown field …"

Scenario files reject unknown keys at every level, so a typo fails at load time. Compare with the [Scenario Format](../reference/scenario-format.md).
