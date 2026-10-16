/**
 * Pause here when a CacheWitness scenario has armed `point` (optionally for
 * this `key`), until the scenario releases it. Returns immediately, without
 * any I/O, unless CAWIT_SYNC_URL is set (and, when NODE_ENV=production, also
 * CAWIT_ALLOW_PRODUCTION=1). Put it right after the database read
 * in a cache refill path and before the cache write.
 */
export declare function syncPoint(point: string, key?: string): Promise<void>;

/**
 * Thrown by syncPoint when the scenario armed the point with action `fail`
 * (fault injection): place the call right before the operation that should
 * fail, e.g. a cache invalidation.
 */
export declare class CawitInjectedError extends Error {}

/** Close the hook's dedicated Redis connection (tests, graceful shutdown). */
export declare function close(): void;

/** Whether the hook is active in this process (read once at load). */
export declare const enabled: boolean;
