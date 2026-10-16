# Security Policy

## Supported versions

CacheWitness is pre-1.0. Security fixes are made on the latest release only.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through GitHub's [private vulnerability reporting](https://github.com/AsyncNomad/CacheWitness/security/advisories/new) for this repository. Include what is affected (runner, hook, or lab scripts), the version, and steps to reproduce.

You should get a first response within seven days. We will agree on a disclosure date with you once a fix is ready.

## Scope and expectations

CacheWitness is a testing tool. The sync hook and the scenario runner are meant for test environments:

- The hook does nothing unless `CAWIT_SYNC_URL` is set, and stays off under `NODE_ENV=production` unless `CAWIT_ALLOW_PRODUCTION=1`. Anyone who can write to the Redis named by `CAWIT_SYNC_URL` can pause or fail requests that reach a sync point; treat that Redis as test infrastructure.
- The runner sends the requests in your scenario to the servers in your scenario and connects to the PostgreSQL and Redis you name. Scenario files are trusted input.
- The runner supports TLS (`https://`, `rediss://`, PostgreSQL `sslmode`) and Redis `AUTH`. PostgreSQL certificates are not verified (libpq `sslmode=require` semantics), and `tls.insecure` turns off verification for Redis and applications; use both only against test servers. Redis passwords are redacted from output.

Reports about the hook turning on, or doing I/O, when it should be off are in scope and especially welcome.
