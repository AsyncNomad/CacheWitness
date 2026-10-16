# cawit-resp

Minimal RESP2/RESP3 frame parser and encoder, and a small async Redis client (one connection, one command at a time), used by [CacheWitness](https://github.com/AsyncNomad/CacheWitness).

It is deliberately small: no pooling, no pub/sub, no cluster support. For a general Redis client use `redis` or `fred`.

The API is not stable before 1.0. Licensed under Apache-2.0.
