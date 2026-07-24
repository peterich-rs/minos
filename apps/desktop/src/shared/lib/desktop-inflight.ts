/**
 * Process-private in-flight / resume bookkeeping.
 * Replaces former `window.__minos*` globals (spec §12.9).
 */

/** Sessions that already received auto-continue resume this boot. */
export const resumedInterruptedSessions = new Set<string>();

/** Sessions with resumeInterruptedSession currently running. */
export const resumeInFlightSessions = new Set<string>();

/**
 * Per-key single-flight for ensureLoaded-style RPCs.
 * Same key+mode reuses the in-flight Promise; generation still guards stale commits.
 */
const loadInflight = new Map<string, Promise<void>>();

export function singleFlightLoad(
  key: string,
  run: () => Promise<void>,
): Promise<void> {
  const existing = loadInflight.get(key);
  if (existing) return existing;
  const p = run().finally(() => {
    if (loadInflight.get(key) === p) {
      loadInflight.delete(key);
    }
  });
  loadInflight.set(key, p);
  return p;
}

/** Drop all load single-flight entries (workspace boundary / tests). */
export function clearLoadInflight(): void {
  loadInflight.clear();
}

/** @deprecated use clearLoadInflight */
export const clearLoadInflightForTests = clearLoadInflight;

/** Clear resume bookkeeping + load single-flight (daemon bootstrap wipe). */
export function clearDesktopInflightState(): void {
  resumedInterruptedSessions.clear();
  resumeInFlightSessions.clear();
  clearLoadInflight();
}
