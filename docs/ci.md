# Running `cawit verify` in CI

`cawit verify` is an ordinary command with a meaningful exit code (`0` pass, `1` a run failed, `2` inconclusive) and a JUnit report, so any CI system can run it. What it needs:

- your application running with the hook enabled (`CAWIT_SYNC_URL` pointing at a Redis the runner can reach),
- the same PostgreSQL and Redis the application uses, reachable from the runner,
- the scenario file and any environment variables it reads (`${env.X}`).

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
      CAWIT_SYNC_URL: redis://127.0.0.1:6379   # the hook in your app reads this
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - run: npm ci && npm run build
      - run: npm run db:migrate && npm run db:seed          # your own setup
      # two instances, so the scenario can read through one and write through the other
      - run: |
          PORT=3000 npm start &
          PORT=3001 npm start &
          npx wait-on http://127.0.0.1:3000/health http://127.0.0.1:3001/health
      - run: cargo install cawit --locked
      - run: cawit verify tests/cache/stale-set.yaml --junit cawit-report.xml
      - uses: actions/upload-artifact@v4
        if: always()
        with: { name: cawit-report, path: cawit-report.xml }
```

Notes:

- **Exit code 2** means some runs were discarded (a sync point was not reached, or a hold timed out). Treat it as a failure of the test setup, not as a pass: usually the hook is not on the path the scenario exercises, the app was started without `CAWIT_SYNC_URL`, or a bundler baked an old copy of the hook into the build.
- `NODE_ENV=production` turns the hook off unless `CAWIT_ALLOW_PRODUCTION=1` is also set. Set it only in the CI job.
- Cold starts make the first requests slow. If the code under test relies on timers, pin the timing in the scenario (`hold_ms`, `within_ms`); see [scenarios.md](scenarios.md#determinism).
- `cargo install` builds from source (a minute or two). Prebuilt binaries are planned.
