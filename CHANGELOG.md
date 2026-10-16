# Changelog

All notable changes to CacheWitness are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/) (before 1.0, minor versions may break compatibility).

## [Unreleased]

### Added

- TLS and authentication: `rediss://` and Redis `AUTH` (user and password) in the runner and the Node hook, `https://` application URLs, PostgreSQL TLS following `sslmode`, and a scenario `tls` section (`ca_file`, `insecure`). The hook reads `CAWIT_SYNC_TLS_INSECURE` and Node's `NODE_EXTRA_CA_CERTS`.
- `${env.X}` in `redis`, `postgres`, and `apps`, to keep passwords out of scenario files.
- `cawit_resp::RedisUrl` and `cawit_resp::redact`.

### Fixed

- A failed `AUTH` or `SELECT`, or a non-numeric database in a Redis URL, is now an error instead of being ignored.
- Redis passwords are no longer printed in error messages (runner and hook).

## [0.1.0] - unreleased

First release of the verify mode: force a cache race in the same order on every run, and prove a fix prevents it.

### Added

- `cawit verify <scenario.yaml>`: runs a scenario N times; per-run output, summary, JSON (`--out`), JUnit (`--junit`), `--show-steps`; exit codes 0 / 1 / 2.
- Scenario format: named apps and request templates, `state` with `cache_key` or `app_view`, hand-written `steps` or a built-in `pattern`. See the [scenario format reference](docs/book/en/src/reference/scenario-format.md).
- Built-in patterns: `stale-set`, `failed-invalidation` (fault injection), `invalidate-before-commit`.
- Checks: `cache_not_stale`, `cache_equals_db`, `cache_absent`, `app_equals_db`, `app_cached_equals_db`; strict or bounded (`within_ms`) verdicts.
- Manipulation check: runs whose sync point was not reached, or whose hold timed out before release, are `NOT_REACHED` and never count as passes.
- `cawit-hook` for Node (no dependencies): `syncPoint(point, key)`, hook protocol v2 (`hold` and `fail` actions), off without `CAWIT_SYNC_URL` and under `NODE_ENV=production` unless `CAWIT_ALLOW_PRODUCTION=1`.
- Crates: `cawit` (CLI), `cawit-verify` (engine, with registries for steps, checks, and patterns), `cawit-resp` (minimal RESP client).

### Verified against

- Twenty CRM (NestJS, TypeORM): stale-set with four candidate fixes.
- Directus 12.5 (Express, knex): stale-set, failed invalidation, invalidate-before-commit; two instances and one.
