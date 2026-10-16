# FAQ

### How is this different from failpoints (RocksDB SyncPoint, fail-rs, Go failpoint)?

Those pause or fail code at named points inside one process, compiled into that process's tests. CacheWitness uses the same idea but coordinates **across processes and servers through Redis**, drives the interleaving with **HTTP requests against the running application**, and ships **cache-specific patterns and verdicts** (stale value, refill, strict or bounded staleness).

### How is it different from Lincheck, Fray, or Coyote?

Those explore thread interleavings inside one JVM or .NET runtime. CacheWitness works at the request level against a real database and Redis, and forces one interleaving per pattern instead of searching all of them.

### Does it find races I don't know about?

No. You choose the pattern and place the hook. What it gives you is a deterministic test for a race you suspect, and a way to tell a real fix from a plausible one. Automatic exploration of orders between several hook points is on the [roadmap](../project/roadmap.md).

### Can it tell me how often the race happens in production?

No. Rates measured under synthetic load vary widely between identical runs (from 2.5× up to about 38× in our measurements) and say little about production. The case studies report them only to judge whether a bug is worth fixing.

### Is the hook safe to leave in production code?

It is designed to be: without `CAWIT_SYNC_URL` it returns before doing any work, and under `NODE_ENV=production` it stays off unless `CAWIT_ALLOW_PRODUCTION=1`. Still, it is test instrumentation; some teams prefer to keep it behind their own feature flag or in a test-only wrapper.

### Do I need two application instances?

No. A held call waits on its own Redis connection, so the competing request can go through the same instance. Two instances are closer to production.

### My application is not written in Node.

The runner does not care what the application is written in. You need a hook for your language; the [protocol](../reference/hook-protocol.md) is a few Redis commands. A Python hook is planned.

### Can it reach a managed Redis or a staging environment with TLS and passwords?

Yes: `rediss://` with `AUTH` (user and password), `https://` application URLs, and PostgreSQL with `sslmode`. Private CAs go in `tls.ca_file` (runner) and `NODE_EXTRA_CA_CERTS` (hook); passwords can come from the environment (`${env.X}`). See [Scenario Format](../reference/scenario-format.md#tls-and-authentication).

### Does it work with MySQL, Redis Cluster, Memcached, or Valkey?

- **Valkey**: yes, it speaks the Redis protocol.
- **MySQL**: not yet; checks read the committed value from PostgreSQL. MySQL support is planned.
- **Redis Cluster**: not yet; the runner and hook use one Redis connection and a single-node keyspace for the protocol. The protocol Redis can be a separate single node even if your cache is a cluster, as long as your `cache_key` checks do not need the cluster (use `app_view` instead).
- **Memcached**: the protocol needs Redis, but your cache can be anything when you judge through `app_view`.

### Why not a proxy that delays cache commands instead of a hook?

We measured it: delaying commands in a connection-level proxy creates head-of-line queues that inflate race rates 10 to 40 times compared with a real slow refill, and it cannot pause at a precise point in the application. See the [project's experiment notes](https://github.com/AsyncNomad/CacheWitness/blob/main/docs/design/experiments-summary.md).

### Why is the result `2` and not `0` when nothing failed?

Because some runs did not test anything. A suite that silently stops reaching its sync point (after a refactor, a bundler change, a config change) must not look green.
