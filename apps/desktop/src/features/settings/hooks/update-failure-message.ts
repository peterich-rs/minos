/**
 * Pure helpers for update failure copy — free of `@/` imports for node:test.
 */

export type RestoreOutcome = {
  restored: boolean;
  error?: string;
};

/** Compose a single user-visible error covering install + optional restore failure. */
export function formatUpdateFailureMessage(
  installError: string,
  restore: RestoreOutcome,
): string {
  if (restore.restored) {
    return installError;
  }
  const detail = restore.error?.trim() || "daemon did not come back online";
  return `${installError} (local daemon restore failed: ${detail})`;
}
