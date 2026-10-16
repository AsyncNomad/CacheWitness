# Getting Started

This page takes an existing Node application with a Redis cache in front of PostgreSQL and adds a test that forces the classic stale-set race. It takes about fifteen minutes.

You need:

- your application, runnable locally as **two instances** (two ports) sharing one PostgreSQL and one Redis (one instance also works; see [Writing a Scenario](guide/writing-scenarios.md#5-one-instance-or-two)),
- an endpoint that reads a cached item and one that updates it,
- Rust (to install the runner) and Node 18 or later.

## 1. Install

```bash
cargo install cawit                 # the runner: a single binary
npm install --save-dev cawit-hook   # the hook: no dependencies
```

> Until v0.1 is published, build the runner from the repository (`cargo build --release`, binary at `target/release/cawit`) and install the hook from `hooks/node`.

## 2. Find the refill path and add the hook

Find the code that runs on a cache miss: it reads the database and then writes the cache. Put one call **between the read and the write**:

```ts
import * as cawit from "cawit-hook";

async function getArticle(id: string) {
  const key = `article:${id}`;
  const cached = await redis.get(key);
  if (cached) return JSON.parse(cached);

  const row = await db.query("select * from articles where id = $1", [id]);
  await cawit.syncPoint("refill:after-read", `/articles/${id}`);   // ← the hook
  await redis.set(key, JSON.stringify(row), "EX", 300);
  return row;
}
```

The first argument names the point; the second is a key the scenario can use to hold only one particular refill. Any string works; here it is the request path, which is easy to write in the scenario.

The call does nothing (no I/O) unless `CAWIT_SYNC_URL` is set. See [The Node Hook](guide/node-hook.md) for cache libraries, bundlers, and the production guard.

## 3. Start the application with the hook enabled

```bash
export CAWIT_SYNC_URL=redis://127.0.0.1:6379   # the same Redis the runner will use
PORT=3000 npm start &
PORT=3001 npm start &
```

## 4. Describe the race

Create `stale-set.yaml`:

```yaml
name: article-stale-set
runs: 20
apps:
  a: http://127.0.0.1:3000
  b: http://127.0.0.1:3001
redis: redis://127.0.0.1:6379
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=app"

requests:
  update:
    method: PATCH
    path: /articles/1
    json: { title: "${value}" }
  read:
    method: GET
    path: /articles/1

state:
  db_query: "select title from articles where id = 1"
  cache_key: "article:1"
  cache_field: title

pattern:
  stale-set:
    reader: a
    writer: b
    write: update
    read: read
    key: /articles/1
```

- `requests` are the HTTP calls the scenario may send. The write request must put `${value}` where the new value goes; each run writes unique values.
- `state` says what to compare: the `title` field of the JSON stored under `article:1`, against `title` in the database. If your cache keys are hashed or values are compressed, read through the application instead with `app_view` ([Scenario Format](reference/scenario-format.md#state)).
- `pattern: stale-set` expands into the steps that force the race ([Race Patterns](guide/patterns.md#stale-set)).

See exactly what will run:

```bash
cawit verify stale-set.yaml --show-steps
```

## 5. Run it

```bash
cawit verify stale-set.yaml
```

Without a fix, every run should fail:

```text
run   1  FAIL
          FAIL not_stale (cache_not_stale): cache=…-1-a db=…-1-b
          FAIL refilled (cache_equals_db): cache=…-1-a db=…-1-b
...
article-stale-set: 20 runs  pass 0  fail 20  not_reached 0  error 0
  not_stale            passed 0/20
  refilled             passed 0/20
```

The exit code is `1`. That is the race, reproduced on demand.

If runs show `NOT_REACHED` instead, the paused request never reached the hook: see [Troubleshooting](guide/troubleshooting.md#not_reached).

## 6. Fix it and prove the fix

Apply a fix (for example a lease, or a generation check; see [Race Patterns](guide/patterns.md#fixes-that-pass-and-fixes-that-do-not)), restart the application, and run the same scenario. A correct fix passes every run and exits `0`. Keep the scenario in your repository and run it in CI ([Running in CI](guide/ci.md)).

## Next steps

- [Core Concepts](guide/concepts.md): sync points, runs, the manipulation check, verdicts.
- [Race Patterns](guide/patterns.md): the three built-in races and which fixes survive them.
- [Case studies](case-studies/directus.md): complete scenarios against Twenty and Directus.
