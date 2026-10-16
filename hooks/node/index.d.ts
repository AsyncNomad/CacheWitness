/**
 * Pause here when a CacheWitness scenario has armed `point` (optionally for
 * this `key`), until the scenario releases it. Returns immediately, without
 * any I/O, unless CAWIT_SYNC_URL is set. Put it right after the database read
 * in a cache refill path and before the cache write.
 */
export declare function syncPoint(point: string, key?: string): Promise<void>;

/** Close the hook's dedicated Redis connection (tests, graceful shutdown). */
export declare function close(): void;
