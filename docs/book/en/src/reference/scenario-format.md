# Scenario Format

A scenario is a YAML file. Unknown keys are errors at every level, so a typo fails at load time instead of being ignored.

```yaml
name: directus-stale-set          # required
runs: 20                          # default 1; --runs overrides
apps:                             # required: named base URLs (plain http://)
  a: http://127.0.0.1:8055
  b: http://127.0.0.1:8056
redis: redis://127.0.0.1:6379     # required: protocol + cache reads
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=directus"

requests: { ... }                 # named request templates
state: { ... }                    # what checks compare
pattern: { ... }                  # either a pattern...
# steps: [ ... ]                  # ...or hand-written steps
```

## Top level

| Key | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Output label and JUnit suite name |
| `runs` | integer | no (1) | Number of runs |
| `apps` | map name → URL | yes | Application servers, `http://` or `https://` |
| `redis` | string | yes | `redis://[[user]:password@]host[:port][/db]`, or `rediss://` for TLS, for the hook protocol and `cache_key` reads; must be the Redis in the application's `CAWIT_SYNC_URL` |
| `postgres` | string | for checks | libpq key=value connection string; `sslmode` is honoured ([TLS](#tls-and-authentication)) |
| `tls` | map | no | TLS options for `rediss://` and `https://` ([below](#tls-and-authentication)) |
| `requests` | map name → [request](#requests) | yes in practice | Request templates |
| `state` | [state](#state) | for checks | What checks compare |
| `pattern` | one-key map | one of | A [pattern](../guide/patterns.md) and its parameters |
| `steps` | list | one of | Hand-written [steps](#steps) |

Exactly one of `pattern` and `steps` must be present.

## TLS and authentication

| Connection | How |
|---|---|
| Redis | `redis://user:password@host` (or `:password@`, or a lone `password@`) sends `AUTH`; `rediss://` uses TLS. User and password are percent-decoded |
| Applications | `https://` URLs in `apps` |
| PostgreSQL | `sslmode` in the connection string: `disable`, `prefer` (default: TLS if the server offers it), `require`. As in libpq for these modes, the server certificate is not verified |

Certificates for Redis and applications are verified against the system trust store (the bundled Mozilla roots when it is empty). For private CAs or self-signed test servers:

```yaml
tls:
  ca_file: certs/ca.pem    # extra trusted CA (PEM), relative to the scenario file
  insecure: false          # true: do not verify certificates (test servers only)
```

Keep passwords out of scenario files: `redis`, `postgres`, and the URLs in `apps` accept `${env.X}`, e.g. `redis: rediss://:${env.REDIS_PASSWORD}@cache:6380`. Passwords in Redis URLs are shown as `***` in all output.

## Requests

| Key | Type | Default | Meaning |
|---|---|---|---|
| `method` | string | `POST` | HTTP method |
| `path` | string | required | Path and query |
| `headers` | map | `{}` | Extra headers |
| `json` | any | none | JSON body, sent with `content-type: application/json` |
| `error_field` | string | none | Treat the response as an error if its JSON body has this top-level field with a non-null value (GraphQL: `errors`) |

A status of 400 or higher is an error. An error ends the run as `ERROR` unless the step sets `allow_error`.

### Variables

`path`, header values, and every string inside `json` may contain `${...}`:

| Variable | Value |
|---|---|
| `${run}` | unique per run: a run id shared by all runs of one invocation, a dash, and the run number |
| `${run_index}` | run number, from 1 |
| `${env.NAME}` | environment variable of the `cawit` process; error if unset |
| `${name}` | an entry of the step's `vars` (whose values may themselves use `${run}` and `${env.X}`) |

An unknown variable is an error. The `arm` step's `key` is rendered the same way. `redis`, `postgres`, and `apps` accept `${env.X}`; `state` values are used literally.

## State

| Key | Type | Meaning |
|---|---|---|
| `db_query` | string | SQL whose first column of the first row is the committed value, as text (`::text` to cast) |
| `cache_key` | string | Redis key of the cache entry |
| `cache_field` | string | Top-level field to take when the entry is a JSON object |
| `app_view` | map | Read through the application instead (below) |

`app_view`:

| Key | Default | Meaning |
|---|---|---|
| `app` | required | App to send the request to |
| `request` | required | Request template |
| `field` | required | JSON pointer into the response body, e.g. `/data/title` |
| `hit_header` | none | Header that tells whether the cache served the response |
| `hit_value` | `HIT` | Value of `hit_header` that means a hit |

## Patterns

See [Race Patterns](../guide/patterns.md). Parameters:

| Parameter | `stale-set` | `failed-invalidation` | `invalidate-before-commit` | Default |
|---|---|---|---|---|
| `reader` | required | | required | |
| `writer` | required | | required | |
| `app` | | required | | |
| `write`, `read` | required | required | required | |
| `value_var` | ✓ | ✓ | ✓ | `value` |
| `point` | ✓ | ✓ | ✓ | `refill:after-read` / `invalidate:before` / `write:before-commit` |
| `key` | ✓ | ✓ | ✓ | any key |
| `hold_ms` | ✓ | | ✓ | `0` |
| `settle_ms` | ✓ | ✓ | ✓ | `100` |
| `within_ms` | ✓ | ✓ | ✓ | strict |
| `reached_timeout_ms` | ✓ | ✓ | ✓ | `10000` |

## Steps

Each step is a one-key map, `{ <kind>: { <params> } }`.

### `http`

| Param | Default | Meaning |
|---|---|---|
| `app` | required | App name |
| `request` | required | Request template name |
| `vars` | `{}` | Extra template variables |
| `async` | none | Run in the background under this task name; collect with `await` |
| `allow_error` | `false` | An error response does not end the run |

### `arm`

| Param | Default | Meaning |
|---|---|---|
| `point` | required | Sync point name |
| `key` | any | Take only a call with this key |
| `action` | `hold` | `hold` (wait until released) or `fail` (throw once) |
| `as` | required | Alias used by `reached` and `release` |

The arm expires after 30 s if nothing takes it.

### `reached`

| Param | Default | Meaning |
|---|---|---|
| `sync` | required | Alias |
| `timeout_ms` | `10000` | How long to wait; then the run is `NOT_REACHED` |

### `release`

| Param | Meaning |
|---|---|
| `sync` | Alias. If the hold already timed out in the application, the run is `NOT_REACHED` |

### `await`, `sleep`

`await: { task: <name> }` waits for a background request (its error ends the run unless it allowed errors). `sleep: { ms: <n> }` waits.

### `expect`

| Param | Default | Meaning |
|---|---|---|
| `name` | required | Label in output and reports |
| `check` | required | A check (below) |
| `within_ms` | strict | Retry every 50 ms; pass if it passes within this time |

## Checks

| Check | Needs | Passes when |
|---|---|---|
| `cache_not_stale` | `cache_key`, `postgres` | the entry is absent or equals the database |
| `cache_equals_db` | `cache_key`, `postgres` | the entry exists and equals the database |
| `cache_absent` | `cache_key` | the entry is absent |
| `app_equals_db` | `app_view`, `postgres` | the application serves the database value |
| `app_cached_equals_db` | `app_view` with `hit_header`, `postgres` | that, and the response was a cache hit |

## End of a run

Whatever happened, each run ends by disarming its sync points, releasing anything still held, and waiting up to 10 s for background requests, so one run never leaves the application blocked for the next.

## A hand-written example

The `stale-set` pattern, written out:

```yaml
steps:
  - http: { app: a, request: update, vars: { value: "${run}-a" } }
  - sleep: { ms: 50 }
  - arm: { point: "refill:after-read", key: /items/articles/1, as: refill }
  - http: { app: a, request: read, async: held_read }
  - reached: { sync: refill, timeout_ms: 10000 }
  - http: { app: b, request: update, vars: { value: "${run}-b" } }
  - release: { sync: refill }
  - await: { task: held_read }
  - sleep: { ms: 100 }
  - expect: { name: not_stale, check: app_equals_db }
  - expect: { name: refilled, check: app_cached_equals_db }
```
