<div align="center">

**English** · [简体中文](docs/readme/README_zh.md) · [한국어](docs/readme/README_ko.md)

# CacheWitness

**Reproduce cache concurrency bugs on demand, and prove your fix works**

**Force the race. Prove the fix.**

</div>

---

## Introduction

   **CacheWitness** (CLI: `cawit`) is an open-source Rust testing tool for PostgreSQL + Redis applications. It **makes a cache race happen in exactly the same order every time, and proves in CI that a fix really prevents it.** In one line: **"Trigger the cache race the same way every time, and prove the fix actually stops it."**

   Cache-aside with Redis in front of the database is the default for web backends, and its races are well known: a reader loads an old row, a writer commits and invalidates, and the reader writes the old row back. The trouble is that these races are timing accidents. They rarely show up in tests, they vanish when you add logging, and **a fix cannot be tested against an accident that may or may not happen.** We measured exactly this on a real application: under the same load, one run left stale values after 6.5% of write bursts and another after 0.17%.

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

> Early stage: the runner (`cawit verify`) and the Node hook (`hooks/node`) work, and `lab/twenty/verify.sh` runs this flow against Twenty CRM ([`lab/twenty/scenarios/stale-set.yaml`](lab/twenty/scenarios/stale-set.yaml)). Nothing is published to crates.io or npm yet.

### 1. Add the sync hook to your refill path

One call, right after the database read and before the cache write. It does nothing unless a test arms it.

```ts
const row = await db.findOne(id);
await cawit.syncPoint("refill:after-read", key);   // no-op unless armed by cawit
await cache.set(key, serialize(row));
```

### 2. Run a scenario

```bash
# Force a stale-set race 20 times; exit code 1 if any run leaves a stale value
cawit verify stale-set.yaml --runs 20 --junit report.xml
```

Each run:

1. writes through server A (commits and invalidates),
2. arms the hook and reads through A: A misses, reads the database, and stops at the hook,
3. writes a newer value through server B (commits and invalidates),
4. releases A, which writes what it read,
5. compares Redis with PostgreSQL,
6. reads once more and checks that the cache refills with the current value (a fix must not stop caching altogether).

A run that never reached the hook is discarded, not counted as a pass.

### 3. Read the result

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

The verdict is simple: does Redis hold what the database committed, and does the cache refill afterwards? When a run fails, CacheWitness explains it with the observed order of events (Redis execution order and PostgreSQL commit order), not with wall-clock timestamps, which proved unreliable in our measurements.

### 4. Why Rust

- **A single static binary:** drops straight into CI runners.
- **The observation engine** (Redis protocol proxy, PostgreSQL logical replication reader) needs predictable latency and bounded memory, and parses untrusted traffic.

---

## Key Features

- **🎯 Deterministic reproduction:** known cache race patterns, triggered in the same order every run
- **✅ Fix verification:** fails before the fix, passes after it, and checks the cache still refills
- **🧪 Scenario library:** stale-set, invalidate-before-commit, failed invalidation, replica-lag refill
- **🔍 Manipulation check:** runs that never reached the hook are discarded
- **🧾 Explanations from order:** failed runs come with the observed sequence of commits, deletes, and writes
- **👀 Secondary: check mode** (no code changes): confirms stale cache state by watching Redis and the PostgreSQL WAL

---

## What We Measured

All experiments ran locally, with the application lab isolated from the network. Details: [`docs/design/experiments-summary.md`](docs/design/experiments-summary.md) (Korean).

- **Forced order on Twenty CRM (20 runs each):** unfixed 20/20 stale; version-compare SET 20/20 stale; lease 0/20 stale but no refill (in-process memoizer); lease + re-read 0/20 stale and 20/20 refilled.
- **Forced order on Directus (20 runs each):** unfixed 20/20 stale; delayed double delete 20/20 stale when the reader is slower than the delay (but 10/10 fine with a fast reader, which is how it slips through ordinary tests); generation check 20/20 pass.
- **Probability instead of order:** the same burst workload left stale values in 6.5% of bursts in one run and 0.17% in another. Delaying cache writes through a proxy inflated the rate up to 10-30x compared with a real slow refill.
- **Detection on a real test suite:** 1,527 of 1,528 changes to cached rows were invalidated (median 1.7ms). Stale state was confirmed reliably in our ground-truth runs (157/157), but deterministic invalidation bugs were rare.

These findings are why CacheWitness focuses on forced reproduction and fix verification.

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

| Document | Contents |
|---|---|
| [`docs/PLAN.md`](docs/PLAN.md) | Development plan (Korean): product definition, design, phase gates, rejected alternatives |
| [`docs/design/experiments-summary.md`](docs/design/experiments-summary.md) | Every experiment so far and what it changed (Korean) |

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

This project is distributed under the **Apache License 2.0**.
