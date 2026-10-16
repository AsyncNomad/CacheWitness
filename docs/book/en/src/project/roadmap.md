# Roadmap

CacheWitness is developed in gated phases: each phase ends with a measurable gate, and the plan is revisited at every gate.

| Version | Focus | Status |
|---|---|---|
| **0.1** | Verify mode: scenarios, three patterns, Node hook, CI reports | done; release pending |
| 0.2 | Fix recipes: documented, verified fixes for each pattern (leases with re-read, versioned invalidation, generation checks); proposals to cache library and ORM maintainers | next |
| 0.3 | Check mode (secondary): confirm stale cache state without code changes by watching Redis and the PostgreSQL WAL; explanations of failed runs from the observed order of events; decoders for compressed values | planned |
| 1.0 | Python hook, GitHub Action, scenario guides per framework, compatibility matrix | planned |

Under consideration, depending on feedback:

- **Interleaving exploration**: run all orders between several sync points automatically, so users pick a pattern and the tool searches the orders.
- **Automatic hooks** for cache libraries (keyv, cache-manager): wrap `set` so no application code changes are needed for the common case.
- **Hook version diagnostics**: hooks record their protocol version, and the runner explains `NOT_REACHED` caused by an outdated hook.
- **Warm-up runs** that are not judged, to keep cold starts out of the results.
- Prebuilt binaries; MySQL; replica-lag refill and list/aggregate cache patterns.

The detailed plan, with gates and rejected alternatives, is in [`docs/PLAN.md`](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/PLAN.md) (Korean).
