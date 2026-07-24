import type { SessionStatus } from "./mock-data";

/**
 * Derive UI session status from daemon session status + optional pending-approval
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
 * Kinds that prove the agent (or user) continued past a parked reverse-request.
 * Historical replay keeps raw `approval/request` frames; user decisions are not
 * always durable events. Progress after an approval card means it was answered.
 */
const APPROVAL_PROGRESS_KINDS = new Set([
  "assistant",
  "text",
  "reasoning",
  "tool",
  "tool_result",
  "tool_error",
  "subagent",
  "user",
]);

export type ApprovalDemoteItem = {
  kind: string;
  seq: number;
  requestId?: string | null;
  approvalMethod?: string | null;
  text: string;
  title?: string | null;
  options?: unknown;
};

/**
 * Demote approval/question cards that are followed by later progress.
 *
 * Without this, reopening a session after plan/permission approve re-shows
 * interactive "Plan approval" cards and re-elevates Attention / needs_approval
 * even though the turn already continued.
 */
export function demoteResolvedApprovalItems<T extends ApprovalDemoteItem>(
  items: readonly T[],
): T[] {
  if (items.length === 0) return items as T[];

  let hasPending = false;
  for (const it of items) {
    if (
      (it.kind === "approval" || it.kind === "question") &&
      Boolean(it.requestId)
    ) {
      hasPending = true;
      break;
    }
  }
  if (!hasPending) return items as T[];

  let maxProgressSeq = Number.NEGATIVE_INFINITY;
  for (const it of items) {
    if (APPROVAL_PROGRESS_KINDS.has(it.kind) && it.seq > maxProgressSeq) {
      maxProgressSeq = it.seq;
    }
  }
  if (!Number.isFinite(maxProgressSeq)) return items as T[];

  let changed = false;
  const out = items.map((it) => {
    if (
      (it.kind !== "approval" && it.kind !== "question") ||
      !it.requestId ||
      it.seq >= maxProgressSeq
    ) {
      return it;
    }
    changed = true;
    const isPlan = it.approvalMethod === "x.ai/exit_plan_mode";
    const isQuestion = it.kind === "question";
    return {
      ...it,
      kind: "status",
      text: isPlan
        ? "Plan approved"
        : isQuestion
          ? "Question answered"
          : "Approval resolved",
      title: it.title ?? (isPlan ? "Plan" : "Resolved"),
      requestId: null,
      options: null,
    };
  });
  return changed ? out : (items as T[]);
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

/**
 * Merge a daemon manager SessionStateChanged status into client UI status.
 *
 * Grok parks on permission/plan while daemon still reports `running` — hold
 * client `needs_approval` only against that running signal. Idle / done /
 * suspended / failed always win so a cleared turn cannot leave a ghost Running
 * after a later transcript demote.
 */
export function nextStatusFromManagerEvent(
  prev: SessionStatus,
  daemonStatus: SessionStatus,
): SessionStatus {
  if (prev === "needs_approval" && daemonStatus === "running") {
    return prev;
  }
  return daemonStatus;
}
