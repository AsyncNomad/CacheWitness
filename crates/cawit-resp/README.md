# cawit-resp

Minimal RESP2/RESP3 frame parser and encoder, a small async Redis client (one connection, one command at a time, with `AUTH`, database selection, and TLS for `rediss://`), and the TLS client configuration shared by the [CacheWitness](https://github.com/AsyncNomad/CacheWitness) runner.

It is deliberately small: no pooling, no pub/sub, no cluster support. For a general Redis client use `redis` or `fred`.

The API is not stable before 1.0. Licensed under Apache-2.0.
