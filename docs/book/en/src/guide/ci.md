# Running in CI

`cawit verify` is an ordinary command with a meaningful exit code and a JUnit report, so any CI system can run it.

| Exit code | Meaning | CI should |
|---|---|---|
| `0` | every run passed | pass |
| `1` | a run failed: the race left a stale value or the cache did not refill | fail |
| `2` | no run failed, but runs were discarded (`NOT_REACHED`, `ERROR`), or the scenario could not run | fail: the test did not test anything |

The job needs:

- your application running with the hook enabled (`CAWIT_SYNC_URL`), usually as two instances,
- the PostgreSQL and Redis the application uses, reachable from the runner,
- the scenario file and the environment variables it reads (`${env.X}`).

## GitHub Actions

```yaml
name: cache-races

on: [pull_request]

jobs:
  verify:
    runs-on: ubuntu-24.04
    services:
      postgres:
        image: postgres:16
        env: { POSTGRES_PASSWORD: postgres }
        ports: ["5432:5432"]
        options: --health-cmd pg_isready --health-interval 2s --health-retries 30
      redis:
        image: redis:7.2
        ports: ["6379:6379"]
    env:
      CAWIT_SYNC_URL: redis://127.0.0.1:6379
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - run: npm ci && npm run build
      - run: npm run db:migrate && npm run db:seed
      - name: Start two instances
        run: |
          PORT=3000 npm start > app-0.log 2>&1 &
          PORT=3001 npm start > app-1.log 2>&1 &
          npx wait-on http://127.0.0.1:3000/health http://127.0.0.1:3001/health
      - run: cargo install cawit --locked
      - run: cawit verify tests/cache/stale-set.yaml --junit cawit-report.xml --out cawit-report.json
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: cawit-report
          path: |
            cawit-report.xml
            cawit-report.json
            app-*.log
```

## GitLab CI

```yaml
cache-races:
  image: node:22
  services:
    - name: postgres:16
      alias: postgres
    - name: redis:7.2
      alias: redis
  variables:
    POSTGRES_PASSWORD: postgres
    CAWIT_SYNC_URL: redis://redis:6379
  script:
    - npm ci && npm run build && npm run db:migrate && npm run db:seed
    - (PORT=3000 npm start &) && (PORT=3001 npm start &) && npx wait-on http://127.0.0.1:3000/health http://127.0.0.1:3001/health
    - curl -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env" && cargo install cawit --locked
    - cawit verify tests/cache/stale-set.yaml --junit cawit-report.xml
  artifacts:
    when: always
    reports:
      junit: cawit-report.xml
```

The scenario's `redis` and `postgres` must point at the service hosts (`redis`, `postgres`) in this case.

## Reports

- `--junit` writes one test case per run. Failed runs are `<failure>`; discarded runs are `<skipped>`, with the reason.
- `--out` writes the full JSON report ([Report Formats](../reference/report-format.md)).

## Tips

- **Pin the number of runs.** 20 runs make a regression obvious and take well under a minute in the case studies. A single run is enough to reproduce, but more runs catch setups where the race is not forced as intended.
- **`NODE_ENV=production`** disables the hook unless `CAWIT_ALLOW_PRODUCTION=1` is set in the CI job.
- **Cold starts** make the first requests slow. If the code under test relies on timers, pin the timing in the scenario ([Race Patterns](patterns.md#timing-parameters)).
- **Bundled builds**: rebuild after upgrading `cawit-hook`, or mark it external ([The Node Hook](node-hook.md#bundlers)).
- **Install time**: `cargo install` builds from source (a minute or two). Cache `~/.cargo` between jobs; prebuilt binaries are planned.
