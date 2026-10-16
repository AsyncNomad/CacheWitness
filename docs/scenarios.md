# Scenario reference

A scenario is a YAML file that `cawit verify` runs several times. Each run forces one interleaving of requests against your application, then judges the result against the database.

```bash
cawit verify scenario.yaml [--runs N] [--out report.json] [--junit report.xml]
cawit verify scenario.yaml --show-steps   # print the steps the scenario runs, then exit
```

Complete examples: [`lab/twenty/scenarios/`](../lab/twenty/scenarios/) and [`lab/directus/scenarios/`](../lab/directus/scenarios/).

## Top level

| Key | Required | Meaning |
|---|---|---|
| `name` | yes | Name used in output and in the JUnit suite |
| `runs` | no (1) | Number of runs; `--runs` overrides it |
| `apps` | yes | Named base URLs of your application servers, e.g. `a: http://127.0.0.1:3000`. Plain `http://` only |
| `redis` | yes | `redis://host:port[/db]` used for the hook protocol and for reading cache entries. Your app's hook must use the same Redis (`CAWIT_SYNC_URL`) |
| `postgres` | for database checks | libpq-style connection string, e.g. `host=127.0.0.1 user=postgres password=postgres dbname=app` |
| `requests` | yes | Named HTTP request templates (below) |
| `state` | for checks | What the checks compare (below) |
| `pattern` | one of these two | A built-in race pattern (below) |
| `steps` | one of these two | Steps written by hand (below) |

Unknown keys are errors everywhere, so a typo fails loudly instead of being ignored.

## Requests

```yaml
requests:
  update:
    method: PATCH                # default POST
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.TOKEN}" }
    json: { title: "${value}" }  # sent as application/json
  read:
    method: GET
    path: /items/articles/1
    headers: { authorization: "Bearer ${env.TOKEN}" }
    error_field: errors          # optional: a response whose JSON has this field is an error
```

A response with status 400 or above is an error, and so is a response whose JSON body has a non-null `error_field` (GraphQL reports errors with status 200). An error ends the run as `ERROR` unless the step allows it.

**Variables** in `path`, header values, and every string inside `json`:

| Variable | Value |
|---|---|
| `${run}` | Unique per run (run id and run number), e.g. `1a14275a3b7-3` |
| `${run_index}` | Run number, from 1 |
| `${env.NAME}` | Environment variable `NAME` of the `cawit` process (error if unset) |
| `${x}` | A step's `vars` entry, e.g. `vars: { value: "${run}-a" }` |

## State

Checks compare one value from the cache side with one value from the database.

```yaml
state:
  db_query: "select title from articles where id = 1"   # first column of the first row, as text
  # Either read the cache entry directly...
  cache_key: "app:article:1"
  cache_field: title          # optional: take this top-level field of a JSON value
  # ...or read through the application:
  app_view:
    app: a
    request: read
    field: /data/title        # JSON pointer into the response body
    hit_header: x-cache-status   # optional: response header that says whether the cache served it
    hit_value: HIT               # default HIT
```

Use `app_view` when cache keys are hashed or values are compressed or encoded, or when what matters is what clients get. `db_query` must return text; cast other types (`select version::text ...`). `cache_key` and `db_query` are used as written (no variables).

## Patterns

A pattern expands into ordinary steps (`--show-steps` prints them), so anything a pattern does can also be written by hand. All patterns end with two expectations:

- `not_stale`: readers get the committed value (`app_equals_db` with `app_view`, else `cache_not_stale`)
- `refilled`: the cache holds the committed value again, so a fix that stops caching altogether fails (`app_cached_equals_db`, else a read followed by `cache_equals_db`)

Write requests must put the value they write in `${value}` (or the variable named by `value_var`); each run writes `${run}-a` and `${run}-b`.

### `stale-set`

A reader misses and reads the old row; a newer write commits and invalidates; the reader then writes what it read.

```yaml
pattern:
  stale-set:
    reader: a                  # app whose refill is held (it also makes the first write)
    writer: b                  # app that writes the newer value meanwhile (may be the same app)
    write: update
    read: read
    key: /items/articles/1     # optional: hold only the refill of this key
    point: refill:after-read   # default
    hold_ms: 0                 # keep the reader held this long after the newer write (a slow reader)
    settle_ms: 100             # wait after the reader finishes before judging
    within_ms: null            # bounded-staleness verdict (see Verdicts)
    reached_timeout_ms: 10000
```

Hook: `syncPoint('refill:after-read', key)` between the database read and the cache write.

### `failed-invalidation`

The write commits but its invalidation fails once; the cache keeps the old value unless the code recovers.

```yaml
pattern:
  failed-invalidation:
    app: a
    write: update
    read: read
    point: invalidate:before   # default
    key: null
    settle_ms: 100
    within_ms: null
```

Hook: `syncPoint('invalidate:before', key)` right before the invalidation. The pattern arms it with action `fail`, so the call throws once; the write request is allowed to fail.

### `invalidate-before-commit`

A write that invalidates inside its transaction leaves a window in which a reader refills with the still-committed old value. Code that invalidates after the commit passes.

```yaml
pattern:
  invalidate-before-commit:
    writer: b
    reader: a
    write: update
    read: read
    point: write:before-commit # default
    key: null
    hold_ms: 0                 # keep the write uncommitted this long before the reader reads
    settle_ms: 100
    within_ms: null
```

Hook: `syncPoint('write:before-commit', key)` at the end of the write transaction, before the commit.

## Steps

Each step is a one-key mapping.

| Step | Parameters | Does |
|---|---|---|
| `http` | `app`, `request`, `vars`, `async` (task name), `allow_error` | Sends a request. With `async` it runs in the background until `await` |
| `arm` | `point`, `key` (default any), `action` (`hold` or `fail`), `as` (alias) | Arms a sync point for one call |
| `reached` | `sync` (alias), `timeout_ms` (10000) | Waits until the armed call arrives. If it does not, the run is `NOT_REACHED` |
| `release` | `sync` | Releases a held call. If the hold already timed out in the app, the run is `NOT_REACHED` |
| `await` | `task` | Waits for a background request |
| `sleep` | `ms` | Waits |
| `expect` | `name`, `check`, `within_ms` | Evaluates a check and records the result |

Checks: `cache_not_stale` (entry absent or equal to the database), `cache_equals_db`, `cache_absent`, `app_equals_db`, `app_cached_equals_db`.

## Verdicts

- **Strict** (default): the check must pass the first time it is evaluated.
- **Bounded** (`within_ms: N`): the check is retried every 50 ms and passes if it passes within N ms. A short TTL, for example, fails a strict verdict and passes a bound longer than the TTL. Which one is right is your service's decision.

## Outcomes and exit codes

| Run | When |
|---|---|
| `PASS` | every expectation passed |
| `FAIL` | an expectation failed |
| `NOT_REACHED` | a sync point was not reached in time, or a hold timed out before release: the forced order did not happen, so the run proves nothing and is discarded |
| `ERROR` | a request or step failed; discarded |

Exit code: `0` all runs passed, `1` any run failed, `2` no failure but some runs were discarded. In JUnit, discarded runs are `skipped`.

## Determinism

A run is deterministic only for the order and the timing the scenario controls. If the code under test relies on timers (a delayed second invalidation, a TTL), pin the relevant durations with `hold_ms` or `within_ms`; otherwise the verdict depends on how long the steps happened to take.

## Hook protocol (v2)

All keys start with `cawit:`. The runner arms with `SET cawit:arm:<point> <token>|<action>|<key or *> PX 30000`. The hook takes the arm atomically (one `EVAL`), pushes to `cawit:reached:<token>`, and then either throws (`fail`) or waits on `cawit:release:<token>` on its own connection (`hold`), pushing to `cawit:timeout:<token>` if the wait times out. Hooks for other languages need only these Redis commands.
