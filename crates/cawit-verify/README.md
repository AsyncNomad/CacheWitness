# cawit-verify

The verify engine of [CacheWitness](https://github.com/AsyncNomad/CacheWitness): loads scenarios, expands built-in race patterns, drives HTTP requests and sync points through the hook protocol over Redis, and judges each run against the database.

Most users want the [`cawit`](https://crates.io/crates/cawit) command line instead. Use this crate to embed the runner or to register your own step kinds, checks, and patterns (`steps::Registry`, `checks::Registry`, `patterns::Registry`).

Documentation: [scenario format](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/book/en/src/reference/scenario-format.md), [hook protocol](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/book/en/src/reference/hook-protocol.md), [extending the engine](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/book/en/src/architecture/extending.md).

The API is not stable before 1.0. Licensed under Apache-2.0.
