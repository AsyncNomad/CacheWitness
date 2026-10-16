# Design Philosophy

## 1. Control the order, don't sample it

A probabilistic stress test asks *"how often does it fail?"*, and the answer changes from run to run: in our measurements, the same workload (write bursts, refill slowed to 50 ms) gave 6.5% in one run and 0.17% in another. A forced interleaving asks *"does it fail in this order?"*, and the answer is the same every time. Only the second kind of test can guard a fix in CI.

## 2. Every run proves it tested something

If the paused request never reached the hook, the race never happened, and a pass would be meaningless. Every run therefore confirms that each armed sync point was reached, and that a held call was still held when the scenario released it. Runs that fail this *manipulation check* are reported as `NOT_REACHED` and never counted as passes. The exit code distinguishes "a run failed" (`1`) from "the test did not test anything" (`2`).

## 3. Judge what clients see

The verdict compares the value your application serves (or the cache entry) with the value the database committed, and then checks that the cache **refills**. The second check matters: a "fix" that stops caching altogether also prevents stale values, and so does an in-process memoizer that keeps serving an old value while Redis looks fine. Both were found this way.

## 4. State the semantics, including time

Whether a briefly stale value is a bug is your service's decision. By default a verdict is strict: no stale value right after the race. `within_ms` makes it bounded: the stale value must disappear within that time. Determinism holds only for the order and timing the scenario controls, so a scenario that tests timer-based code pins the relevant durations.

## 5. Small surface in the application

The hook is one function call with no dependencies, does no I/O unless enabled, and speaks a tiny protocol over Redis lists. The application can run in any language and on any host that reaches the same Redis; everything else (requests, ordering, verdicts) lives in the scenario and the runner.

## 6. Extensible core

Step kinds, checks, and patterns are registries. The built-in patterns expand into ordinary steps, so nothing a pattern does is hidden, and new patterns or checks plug in without changing the runner.

## Why Rust

- **A single binary, independent of the application's language.** The runner drops into any CI image without a Node or Python runtime, while hooks stay small per-language libraries.
- **Honestly, the runner itself would work in any language**: it mostly sends requests and waits on Redis. Rust pays off in the planned observation engine (a Redis protocol proxy and a PostgreSQL logical replication reader), which needs predictable latency and bounded memory and parses untrusted traffic.
