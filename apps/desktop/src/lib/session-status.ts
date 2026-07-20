import type { SessionStatus } from "./mock-data";

/**
 * Derive UI session status from daemon thread status + optional pending-approval
 * signal from transcript reverse-requests.
 *
 * Daemon never reports `needs_approval` (Grok parks on permission/plan while
 * still `Running`). The UI elevates status after a transcript peek finds a
 * pending approval item.
 *
 * `pendingApproval`:
 * - `true`  → force `needs_approval`
 * - `false` → trust daemon status (clear client elevation) — only pass this when
 *   the signal is high-confidence (append poll continuity, resolveApproval,
 *   or daemon left `running`)
 * - `undefined` → unknown / ambiguous peek — hold prior `needs_approval` while
 *   daemon still says `running` so quiet polls cannot thrash the pill/banner
 */
export function deriveSessionStatus(
  daemonStatus: SessionStatus,
  options: {
    prevStatus?: SessionStatus;
    pendingApproval?: boolean;
  } = {},
): SessionStatus {
  const { prevStatus, pendingApproval } = options;
  if (pendingApproval === true) return "needs_approval";
  if (pendingApproval === false) return daemonStatus;
  // Ambiguous: hold elevation only while daemon still looks live-running.
  if (prevStatus === "needs_approval" && daemonStatus === "running") {
    return "needs_approval";
  }
  return daemonStatus;
}

/** True when transcript tail still has an unresolved approval reverse-request. */
export function transcriptHasPendingApproval(
  items: ReadonlyArray<{ kind: string; requestId?: string | null }>,
): boolean {
  return items.some(
    (it) =>
      (it.kind === "approval" || it.kind === "question") && Boolean(it.requestId),
  );
}

/**
 * Apply derived status to a freshly mapped daemon session list.
 * `pendingById` may be partial — missing ids keep preserve semantics.
 */
export function withDerivedSessionStatuses<
  T extends { id: string; status: SessionStatus },
>(
  daemonSessions: T[],
  prevStatuses: ReadonlyMap<string, SessionStatus> | undefined,
  pendingById?: ReadonlyMap<string, boolean>,
): T[] {
  return daemonSessions.map((s) => ({
    ...s,
    status: deriveSessionStatus(s.status, {
      prevStatus: prevStatuses?.get(s.id),
      // Map.get returns undefined for missing keys — preserve semantics.
      pendingApproval: pendingById?.has(s.id)
        ? pendingById.get(s.id)
        : undefined,
    }),
  }));
}

/**
 * How loadTranscript should mutate client-derived needs_approval.
 * - elevate-only: tail peeks during conversation poll (never demote; window can miss)
 * - sync: open transcript / append poll with continuity (may demote when clear)
 */
export type ApprovalStatusPolicy = "elevate-only" | "sync";

export function nextSessionStatusAfterTranscript(options: {
  current: SessionStatus;
  hasPendingApproval: boolean;
  policy: ApprovalStatusPolicy;
}): SessionStatus {
  const { current, hasPendingApproval, policy } = options;
  if (hasPendingApproval) return "needs_approval";
  if (policy === "elevate-only") {
    // Keep elevated attention until a high-confidence path clears it.
    return current;
  }
  // sync: drop client elevation when transcript no longer has pending cards.
  if (current === "needs_approval") return "running";
  return current;
}
