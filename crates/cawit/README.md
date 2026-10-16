# cawit

Command-line runner for [CacheWitness](https://github.com/AsyncNomad/CacheWitness): force a cache race (PostgreSQL + Redis) in the same order on every run, and prove in CI that a fix prevents it.

```bash
cargo install cawit
cawit verify stale-set.yaml --runs 20 --junit report.xml
cawit verify stale-set.yaml --show-steps   # print the steps a pattern expands to
```

A scenario names a built-in pattern (`stale-set`, `failed-invalidation`, `invalidate-before-commit`) or lists steps by hand. The application calls a one-line sync hook in its cache refill path (Node: the [`cawit-hook`](https://www.npmjs.com/package/cawit-hook) package).

Exit codes: `0` every run passed, `1` a run failed, `2` inconclusive (a run never reached its sync point, or errored).

- Scenario format: [docs/scenarios.md](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/scenarios.md)
- Project overview: [README](https://github.com/AsyncNomad/CacheWitness#readme)

For test environments only. Licensed under Apache-2.0.
