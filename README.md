<div align="center">

**English** · [简体中文](docs/readme/README_zh.md) · [한국어](docs/readme/README_ko.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/logo/cawit-logo-dark.svg">
  <img src="docs/assets/logo/cawit-logo-light.svg" alt="cawit: cache race verifier" width="460">
</picture>

# CacheWitness

**Reproduce cache concurrency bugs on demand, and prove your fix works**

**Force the race. Prove the fix.**

</div>

<p align="center">
  <a href="https://asyncnomad.github.io/CacheWitness/">
    <img src="https://img.shields.io/badge/📖_Project-Documentation-blue?style=for-the-badge" alt="Documentation">
  </a>
  &nbsp;
  <a href="https://asyncnomad.github.io/CacheWitness/api/cawit_verify/index.html">
    <img src="https://img.shields.io/badge/⚙️_Rust-API_Docs-orange?style=for-the-badge&logo=rust" alt="Rust API Docs">
  </a>
</p>

<!-- Add after publishing v0.1:
  <a href="https://crates.io/crates/cawit"><img src="https://img.shields.io/crates/v/cawit?logo=rust&label=crates.io" alt="crates.io"></a>
  <a href="https://www.npmjs.com/package/cawit-hook"><img src="https://img.shields.io/npm/v/cawit-hook?logo=npm&label=cawit-hook" alt="npm"></a>
  <a href="https://docs.rs/cawit-verify"><img src="https://img.shields.io/docsrs/cawit-verify?logo=docsdotrs&label=docs.rs" alt="docs.rs"></a>
-->
<p align="center">
  <a href="https://github.com/AsyncNomad/CacheWitness/actions/workflows/ci.yml"><img src="https://github.com/AsyncNomad/CacheWitness/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust 1.85+">
</p>

---

## Introduction

   **CacheWitness** (CLI: `cawit`) is an open-source Rust testing tool for PostgreSQL + Redis applications. It **makes a cache race happen in exactly the same order every time, and proves in CI that a fix really prevents it.** In one line: **"Trigger the cache race the same way every time, and prove the fix actually stops it."**

   Cache-aside with Redis in front of the database is the default for web backends, and its races are well known: a reader loads an old row, a writer commits and invalidates, and the reader writes the old row back. The trouble is that these races are timing accidents. They rarely show up in tests, they vanish when you add logging, and **a fix cannot be tested against an accident that may or may not happen.** We measured exactly this on a real application: under identical conditions (write bursts, with the refill slowed to 50 ms), one run left stale values after 6.5% of bursts and another after 0.17%.

   **CacheWitness solves this by controlling the order instead of hoping for it.** A small sync hook in your cache refill path pauses a reader right after its database read. The scenario runner then commits and invalidates from another request, releases the reader, and checks Redis against the database. The race happens on every run, so the test fails every time before the fix and passes every time after it. CacheWitness is built on a clear philosophy: **"Don't wait for the race. Cause it, and prove the fix."**

   It is meant for **cache library and ORM authors, and teams that write their own cache code**, who want regression tests for cache invalidation logic that do not flake.

---

## The Problem

| Race | What happens | Typical "fix" that is not enough |
|---|---|---|
| Stale-set race | Reader reads old value → writer commits and DELs → reader SETs the old value | Compare versions before SET: after the DEL there is nothing to compare against |
| Invalidate before commit | DEL runs inside the transaction → another request refills with the old value before the commit | Delete twice with a delay: narrows the window, does not close it |
| Failed invalidation | DEL fails (network, permissions) and is not retried | Short TTL: bounds the damage, does not fix it |
| Replica-lag refill | After the DEL, a lagging replica supplies the old value | Read from the primary on miss: only if every refill path does it |

The safe patterns are known (leases, versioned invalidation markers), and **so are their pitfalls.** In our first real-application experiment, two plausible fixes looked correct and were not: a version-compare SET did nothing, and a lease without re-reading left an in-process memoizer serving the old value for 10 seconds. Only a test that forces the race shows the difference.

---

## Quick Start

> v0.1 is not published yet. Until it is, build the runner with `cargo build --release` and use the hook from [`hooks/node`](hooks/node). Working examples against real applications: [`lab/twenty/`](lab/twenty/) (Twenty CRM) and [`lab/directus/`](lab/directus/) (Directus).

### 1. Install

```bash
cargo install cawit                 # the runner, a single binary
npm install --save-dev cawit-hook   # the hook, no dependencies
```

### 2. Add the sync hook to your refill path

One call, right after the database read and before the cache write. It does nothing unless `CAWIT_SYNC_URL` is set and a test arms it (and stays off under `NODE_ENV=production` unless `CAWIT_ALLOW_PRODUCTION=1`).

```ts
import * as cawit from "cawit-hook";

const row = await db.findOne(id);
await cawit.syncPoint("refill:after-read", key);
await cache.set(key, serialize(row));
```

### 3. Describe the race and run it

```yaml
# stale-set.yaml
name: stale-set
runs: 20
apps: { a: "http://127.0.0.1:3000", b: "http://127.0.0.1:3001" }
redis: redis://127.0.0.1:6379
postgres: "host=127.0.0.1 user=postgres password=postgres dbname=app"
requests:
  update: { method: PATCH, path: /articles/1, json: { title: "${value}" } }
  read:   { method: GET,   path: /articles/1 }
state:
  db_query: "select title from articles where id = 1"
  app_view: { app: a, request: read, field: /title, hit_header: x-cache }
pattern:
  stale-set: { reader: a, writer: b, write: update, read: read, key: /articles/1 }
```

```bash
# start the app with CAWIT_SYNC_URL=redis://127.0.0.1:6379, then:
cawit verify stale-set.yaml --junit report.xml   # exit 1 if any run leaves a stale value
cawit verify stale-set.yaml --show-steps          # see what the pattern does
```

Each run:

1. writes through server A (commits and invalidates),
2. arms the hook and reads through A: A misses, reads the database, and stops at the hook,
3. writes a newer value through B (commits and invalidates),
4. releases A, which writes what it read,
5. checks that readers get what the database committed,
6. checks that the cache refills with the current value (a fix must not stop caching altogether).

A run that never reached the hook, or whose hold timed out, is discarded, not counted as a pass. Patterns, steps, and verdicts: [Scenario Format](docs/book/en/src/reference/scenario-format.md) and [Race Patterns](docs/book/en/src/guide/patterns.md).

### 4. Read the result

```
stale-set (20 runs, every run reached the hook)
  before the fix                    stale 20/20                  FAIL
  version-compare SET               stale 20/20                  FAIL  nothing to compare after DEL
  lease                             stale  0/20, refilled  0/20  FAIL  old value served from memory
  lease + re-read on rejection      stale  0/20, refilled 20/20  PASS
```

---

## Design Philosophy

### 1. Control the order, don't sample it

Probabilistic stress tests ask "how often does it fail?", and the answer changes from run to run. A forced interleaving asks "does it fail in this order?", and the answer is the same every time. That is what makes it usable as a CI regression test.

### 2. Every run proves it tested something

A hook that was never reached means the race never happened, and a pass would be meaningless. Every run records that the hook was reached; runs that were not are discarded (manipulation check).

### 3. Judge by state, explain by order

The verdict is simple: does Redis hold what the database committed, and does the cache refill afterwards? When a run fails, CacheWitness will explain it (planned) with the observed order of events (Redis execution order and PostgreSQL commit order), not with wall-clock timestamps, which proved unreliable in our measurements.

### 4. Why Rust

- **A single binary, independent of the app's language:** the runner drops into any CI runner without a Node or Python runtime, while hooks stay small per-language libraries.
- **Honestly, the runner itself would work in any language:** it mostly sends requests and waits on Redis. Rust pays off in the planned observation engine (Redis protocol proxy, PostgreSQL logical replication reader), which needs predictable latency and bounded memory and parses untrusted traffic.

---

## Key Features

- **🎯 Deterministic reproduction:** known cache race patterns, triggered in the same order every run
- **✅ Fix verification:** fails before the fix, passes after it, and checks the cache still refills
- **🧪 Built-in patterns:** stale-set, failed invalidation (fault injection), invalidate-before-commit; replica-lag refill planned
- **🔍 Manipulation check:** runs that never reached the hook are discarded
- **🧾 Explanations from order (planned):** failed runs come with the observed sequence of commits, deletes, and writes. Today a failed run reports what readers got and what the database holds
- **👀 Secondary: check mode** (no code changes): confirms stale cache state by watching Redis and the PostgreSQL WAL

---

## What We Measured

All experiments ran locally, with the application lab isolated from the network. Details: [`docs/design/experiments-summary.md`](docs/design/experiments-summary.md) (Korean).

- **Forced order on Twenty CRM (20 runs each):** unfixed 20/20 stale; version-compare SET 20/20 stale; lease 0/20 stale but no refill (in-process memoizer); lease + re-read 0/20 stale and 20/20 refilled.
- **Forced order on Directus (20 runs each):** unfixed 20/20 stale; delayed double delete 20/20 stale when the reader is slower than the delay (but 10/10 fine with a fast reader, which is how it slips through ordinary tests); generation check 20/20 pass.
- **Probability instead of order:** with the refill slowed to 50 ms, the same burst workload left stale values in 6.5% of bursts in one run and 0.17% in another. Without the slowdown, no stale value outlived a burst (0 of about 1,100). Delaying cache writes through a proxy inflated the rate up to 10-30x compared with a real slow refill.
- **Detection on a real test suite:** 1,527 of 1,528 changes to cached rows were invalidated (median 1.7ms). Stale state was confirmed reliably in our ground-truth runs (157/157), but deterministic invalidation bugs were rare.

These findings are why CacheWitness focuses on forced reproduction and fix verification.

---

## How It Relates to Existing Tools

Stopping code at a named point to force an order is not new. What CacheWitness adds is where the point lives and what it judges.

| Tool | What it does | Difference |
|---|---|---|
| RocksDB SyncPoint, fail-rs (TiKV), failpoint (Go) | Sync and fault points compiled into one process | CacheWitness coordinates across processes and servers through Redis, driven by HTTP requests |
| Lincheck, Fray (JVM), Coyote (.NET) | Explore thread interleavings inside one runtime | CacheWitness works at the request level against a real database and Redis |
| Jepsen | Black-box consistency testing of distributed databases under faults | CacheWitness targets the application's cache logic, with cache-specific patterns and verdicts |

---

## Scope

| Area | First public release (v0.x) | Later |
|---|---|---|
| Hooks | Node/TS | Python, then others |
| Scenarios | stale-set, invalidate-before-commit, failed invalidation | replica-lag refill, list/aggregate caches |
| Database / cache | PostgreSQL, Redis/Valkey single node | MySQL, Redis Cluster |
| check mode (secondary) | after the verify engine | — |

**What CacheWitness does not do:**

- It does not find races it was not told about: verify is a regression test for known orders.
- It does not estimate how often a race happens in production.
- It does not fix your cache or replace it.

---

## Documentation

The full documentation is at **[asyncnomad.github.io/CacheWitness](https://asyncnomad.github.io/CacheWitness/)** (English, [한국어](https://asyncnomad.github.io/CacheWitness/ko/), [简体中文](https://asyncnomad.github.io/CacheWitness/zh/)). Sources live in [`docs/book/`](docs/book/).

| Start here | |
|---|---|
| [Getting Started](docs/book/en/src/getting-started.md) | Add the hook, write a scenario, reproduce the race, prove the fix |
| [Race Patterns](docs/book/en/src/guide/patterns.md) | The built-in races, with diagrams, and which fixes survive them |
| [Scenario Format](docs/book/en/src/reference/scenario-format.md) | Every key, step, check, and pattern parameter |
| [Hook Protocol](docs/book/en/src/reference/hook-protocol.md) | The Redis protocol, for hooks in other languages |
| [Case studies](docs/book/en/src/case-studies/directus.md) | Twenty CRM and Directus, with results |
| [Rust API](https://asyncnomad.github.io/CacheWitness/api/cawit_verify/index.html) | rustdoc for `cawit-verify` and `cawit-resp` |
| [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Changelog](CHANGELOG.md) | |
| [`docs/PLAN.md`](docs/PLAN.md), [`docs/design/`](docs/design/) | Development plan, design notes, and experiment records (Korean) |


---

## ⚠️ Disclaimer & Liability

**Please read carefully.**

**CacheWitness is currently a personal project at the design and prototype stage.**

This software is provided **"as is", without warranty of any kind**, express or implied, including but not limited to the warranties of merchantability, fitness for a particular purpose, and non-infringement. By using this software, you agree to the following:
1. **Use at your own risk:** You are solely responsible for determining whether using or redistributing this software is appropriate.
2. **No liability:** In no event shall the authors or copyright holders be liable for any claim, damages, or other liability arising from or in connection with the use of this software.
3. **Test environments only:** The sync hook and scenario runner are for test environments. Never arm hooks in production.

---

## License

This project is distributed under the **Apache License 2.0**, except for the lab patches under `lab/` that modify other projects (Twenty, Directus). Those are licensed under the respective project's license; see [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md). Twenty and Directus are trademarks of their owners; CacheWitness is not affiliated with or endorsed by them.
