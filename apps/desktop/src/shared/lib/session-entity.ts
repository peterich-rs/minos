/**
 * L4 SessionEntity — process-wide light truth for session status /
 * hasPendingApproval. All UI status mutations must go through merge helpers
 * here (or store wrappers that call them); list caches only project Entity.
 *
 * hasPendingApproval is the fallback truth when Transcript is missing/evicted:
 * never demote needs_approval by scanning a missing transcript window.
 *
 * List/RPC hydrates are *samples* — they must not clobber fresher live
 * lifecycle without a newer lastTsMs (or an authoritative manager path).
 */

import type { AgentRuntime, SessionStatus } from "./mock-data.ts";
import type { ApprovalStatusPolicy } from "./session-status.ts";

export type SessionEntity = {
  sessionId: string;
  conversationId: string;
  conversationTitle?: string;
  agent: string;
  shortId: string;
  /** UI status after elevation (may be needs_approval). */
  status: SessionStatus;
  /** Last daemon lifecycle label (never needs_approval). */
  daemonStatus: SessionStatus;
  model: string;
  parentId?: string;
  summary: string;
  messageCount: number;
  firstTsMs?: number;
  lastTsMs?: number;
  needsContinue?: boolean;
  /**
   * Pending approval/question snapshot from list/ingest/resolve.
   * Fallback truth when Transcript is not in working set.
   */
  hasPendingApproval: boolean;
  updatedAtMs: number;
};

/** Daemon never emits needs_approval; coerce legacy wire values. */
export function asDaemonStatus(status: SessionStatus | string): SessionStatus {
  if (status === "needs_approval") return "running";
  if (
    status === "idle" ||
    status === "running" ||
    status === "suspended" ||
    status === "failed" ||
    status === "done"
  ) {
    return status;
  }
  return "idle";
}

export function isTerminalDaemonStatus(status: SessionStatus): boolean {
  return status === "done" || status === "failed";
}

/**
 * UI status from pending flag + daemon lifecycle.
 * hasPendingApproval always wins elevation.
 */
export function entityUiStatus(
  hasPendingApproval: boolean,
  daemonStatus: SessionStatus,
): SessionStatus {
  if (hasPendingApproval) return "needs_approval";
  return asDaemonStatus(daemonStatus);
}

/** Live for rail runningCount / recovery polls (UI status). */
export function entityIsLive(e: SessionEntity): boolean {
  if (e.parentId) return false;
  return (
    e.status === "running" ||
    e.status === "needs_approval" ||
    (e.status === "suspended" && Boolean(e.needsContinue))
  );
}

/** Counts toward conversation.runningCount (agent actively working). */
export function entityCountsAsRunning(e: SessionEntity): boolean {
  if (e.parentId) return false;
  return e.status === "running" || e.status === "needs_approval";
}

/**
 * Counts toward conversation.approvalCount / needs_you rail badge
 * (approvals + recovery pause — matches prior Inspector recount).
 */
export function entityCountsAsApproval(e: SessionEntity): boolean {
  if (e.parentId) return false;
  return (
    e.status === "needs_approval" ||
    e.status === "suspended" ||
    e.hasPendingApproval
  );
}

export type SessionEntitySeed = {
  id: string;
  conversationId: string;
  conversationTitle?: string;
  agent: string;
  shortId: string;
  /**
   * Lifecycle label from daemon/list. Prefer raw daemon labels; never pass
   * UI-elevated `needs_approval` (use `daemonStatusFromEntity` when reseeding).
   */
  status: SessionStatus;
  model: string;
  parentId?: string;
  summary: string;
  messageCount?: number;
  firstTsMs?: number;
  lastTsMs?: number;
  needsContinue?: boolean;
};

export type MergeSessionEntityOptions = {
  /**
   * High-confidence pending signal.
   * - true → set hasPendingApproval
   * - false → clear only when policy allows (sync)
   * - omit → keep previous flag
   */
  pendingApproval?: boolean;
  /**
   * elevate-only: never clear hasPendingApproval on false peeks.
   * sync: allow clear when pendingApproval===false.
   * Default: preserve when pendingApproval omitted; sync when false.
   */
  approvalPolicy?: ApprovalStatusPolicy;
  /**
   * How much to trust `seed.status` as lifecycle.
   * - `sample` (default): list/transcript hydrate — anti-stale merge
   * - `authoritative`: manager push / local action success — always apply
   */
  lifecycleSource?: "sample" | "authoritative";
  /** Override updatedAtMs (tests). */
  nowMs?: number;
};

/**
 * Seed lifecycle from Entity.daemonStatus — never feed UI `status` back into
 * merge (needs_approval would coerce to running and erase idle parks).
 */
export function daemonStatusFromEntity(
  entity: SessionEntity | undefined,
  fallback: SessionStatus = "idle",
): SessionStatus {
  if (!entity) return asDaemonStatus(fallback);
  return asDaemonStatus(entity.daemonStatus);
}

/**
 * Merge sample (list/RPC) lifecycle into previous Entity daemon status.
 * Samples lag; manager/actions use `authoritative` and skip this.
 */
export function mergeSampleDaemonStatus(
  prev: SessionEntity | undefined,
  seedDaemon: SessionStatus,
  seed: Pick<SessionEntitySeed, "needsContinue" | "lastTsMs">,
): { daemonStatus: SessionStatus; needsContinue: boolean | undefined } {
  const seedNeeds = seed.needsContinue;
  if (!prev) {
    return {
      daemonStatus: seedDaemon,
      needsContinue: seedNeeds,
    };
  }

  const prevDaemon = asDaemonStatus(prev.daemonStatus);

  // Terminal is sticky against non-terminal list pages (no resurrection).
  if (isTerminalDaemonStatus(prevDaemon) && !isTerminalDaemonStatus(seedDaemon)) {
    return {
      daemonStatus: prevDaemon,
      needsContinue: prev.needsContinue,
    };
  }

  // Explicit terminal from sample is accepted (session closed on server).
  if (isTerminalDaemonStatus(seedDaemon)) {
    return {
      daemonStatus: seedDaemon,
      needsContinue: false,
    };
  }

  // Auto-continue recovery: live running must not bounce to Paused from a
  // pre-resume list snapshot (suspended + needsContinue).
  if (
    prevDaemon === "running" &&
    !prev.needsContinue &&
    seedDaemon === "suspended" &&
    seedNeeds === true
  ) {
    return {
      daemonStatus: "running",
      needsContinue: false,
    };
  }

  // Prefer newer activity clock when both sides report it.
  const prevTs = prev.lastTsMs ?? 0;
  const seedTs = seed.lastTsMs ?? 0;
  if (prevTs > 0 && seedTs > 0 && seedTs < prevTs) {
    // Exception: authoritative turn-end from list (idle/done/failed) must be
    // able to demote optimistic Desktop `running` even when the send path
    // stamped lastTsMs slightly ahead of the daemon's last_activity clock.
    // Without this, a missed manager idle event + livePush (no poll) leaves
    // the rail on Running forever while SQLite is already idle.
    const seedIsTurnEnd =
      seedDaemon === "idle" ||
      seedDaemon === "done" ||
      seedDaemon === "failed";
    if (!(prevDaemon === "running" && seedIsTurnEnd && !prev.needsContinue)) {
      return {
        daemonStatus: prevDaemon,
        needsContinue: prev.needsContinue,
      };
    }
  }

  // Live turn after continue: stale *suspended* list without a newer clock
  // must not demote (manager SessionStateChanged is the demote path).
  // Idle/done/failed from list are accepted as turn-end reconciliation.
  if (
    prevDaemon === "running" &&
    !prev.needsContinue &&
    seedDaemon === "suspended" &&
    !(seedTs > prevTs)
  ) {
    return {
      daemonStatus: "running",
      needsContinue: false,
    };
  }

  return {
    daemonStatus: seedDaemon,
    needsContinue: seedNeeds !== undefined ? seedNeeds : prev.needsContinue,
  };
}

/**
 * @deprecated Use mergeSampleDaemonStatus via mergeSessionEntity.
 * Kept name for call-site clarity in tests.
 */
export function coerceListSeedAgainstLiveEntity(
  prev: SessionEntity | undefined,
  seed: SessionEntitySeed,
): SessionEntitySeed {
  const seedDaemon = asDaemonStatus(seed.status);
  const merged = mergeSampleDaemonStatus(prev, seedDaemon, seed);
  return {
    ...seed,
    status: merged.daemonStatus,
    needsContinue: merged.needsContinue,
  };
}

/**
 * Merge a daemon/list seed + optional pending signal into SessionEntity.
 * Sole pure writer for status/hasPendingApproval derivation.
 */
export function mergeSessionEntity(
  prev: SessionEntity | undefined,
  seed: SessionEntitySeed,
  opts: MergeSessionEntityOptions = {},
): SessionEntity {
  const source = opts.lifecycleSource ?? "sample";
  const rawDaemon = asDaemonStatus(seed.status);

  let daemonStatus: SessionStatus;
  let needsContinue: boolean | undefined;

  if (source === "authoritative") {
    daemonStatus = rawDaemon;
    needsContinue =
      seed.needsContinue !== undefined
        ? seed.needsContinue
        : prev?.needsContinue;
  } else {
    const merged = mergeSampleDaemonStatus(prev, rawDaemon, seed);
    daemonStatus = merged.daemonStatus;
    needsContinue = merged.needsContinue;
  }

  let hasPendingApproval = prev?.hasPendingApproval ?? false;
  if (opts.pendingApproval === true) {
    hasPendingApproval = true;
  } else if (opts.pendingApproval === false) {
    const policy = opts.approvalPolicy ?? "sync";
    if (policy !== "elevate-only") {
      hasPendingApproval = false;
    }
  }

  return {
    sessionId: seed.id,
    conversationId: seed.conversationId || prev?.conversationId || "",
    conversationTitle: seed.conversationTitle ?? prev?.conversationTitle,
    agent: seed.agent || prev?.agent || "codex",
    shortId: seed.shortId || prev?.shortId || seed.id.slice(0, 8),
    daemonStatus,
    model: seed.model || prev?.model || "",
    parentId: seed.parentId ?? prev?.parentId,
    summary: seed.summary ?? prev?.summary ?? "",
    messageCount: seed.messageCount ?? prev?.messageCount ?? 0,
    firstTsMs: seed.firstTsMs ?? prev?.firstTsMs,
    lastTsMs: seed.lastTsMs ?? prev?.lastTsMs,
    needsContinue,
    hasPendingApproval,
    status: entityUiStatus(hasPendingApproval, daemonStatus),
    updatedAtMs: opts.nowMs ?? Date.now(),
  };
}

/**
 * Patch fields on an existing entity (or create a minimal shell).
 * Recomputes status from hasPendingApproval + daemonStatus after merge.
 * Patches are authoritative (manager / local actions).
 */
export function patchSessionEntity(
  prev: SessionEntity | undefined,
  sessionId: string,
  patch: Partial<Omit<SessionEntity, "sessionId" | "status">> & {
    status?: SessionStatus;
  },
  opts: { nowMs?: number } = {},
): SessionEntity {
  const base: SessionEntity = prev ?? {
    sessionId,
    conversationId: "",
    agent: "codex",
    shortId: sessionId.slice(0, 8),
    status: "idle",
    daemonStatus: "idle",
    model: "",
    summary: "",
    messageCount: 0,
    hasPendingApproval: false,
    updatedAtMs: 0,
  };

  const hasPendingApproval =
    patch.hasPendingApproval !== undefined
      ? patch.hasPendingApproval
      : base.hasPendingApproval;

  // Prefer explicit daemonStatus; if caller passes status as lifecycle
  // (not needs_approval), treat it as daemon label.
  let daemonStatus = base.daemonStatus;
  if (patch.daemonStatus !== undefined) {
    daemonStatus = asDaemonStatus(patch.daemonStatus);
  } else if (patch.status !== undefined && patch.status !== "needs_approval") {
    daemonStatus = asDaemonStatus(patch.status);
  }

  const next: SessionEntity = {
    ...base,
    ...patch,
    sessionId,
    hasPendingApproval,
    daemonStatus,
    status: entityUiStatus(hasPendingApproval, daemonStatus),
    updatedAtMs: opts.nowMs ?? Date.now(),
  };
  return next;
}

/**
 * Apply manager lifecycle without clearing hasPendingApproval.
 * When pending is set, UI stays needs_approval regardless of daemon running.
 */
export function applyManagerLifecycleToEntity(
  prev: SessionEntity | undefined,
  sessionId: string,
  daemonStatus: SessionStatus,
  extras?: { lastTsMs?: number; nowMs?: number },
): SessionEntity {
  return patchSessionEntity(
    prev,
    sessionId,
    {
      daemonStatus: asDaemonStatus(daemonStatus),
      lastTsMs: extras?.lastTsMs,
      // Manager lifecycle is not auto-continue recovery.
      ...(asDaemonStatus(daemonStatus) === "running"
        ? { needsContinue: false }
        : {}),
    },
    { nowMs: extras?.nowMs },
  );
}

/** Project Entity → list row shape used by Sessions / Inspector / Attention. */
export function projectSessionFromEntity(e: SessionEntity): {
  id: string;
  conversationId: string;
  conversationTitle?: string;
  agent: AgentRuntime;
  shortId: string;
  status: SessionStatus;
  model: string;
  parentId?: string;
  summary: string;
  needsContinue?: boolean;
  firstTsMs?: number;
  lastTsMs?: number;
  messageCount?: number;
} {
  return {
    id: e.sessionId,
    conversationId: e.conversationId,
    conversationTitle: e.conversationTitle,
    agent: (e.agent as AgentRuntime) || "codex",
    shortId: e.shortId,
    status: e.status,
    model: e.model,
    parentId: e.parentId,
    summary: e.summary,
    needsContinue: Boolean(e.needsContinue),
    firstTsMs: e.firstTsMs,
    lastTsMs: e.lastTsMs,
    messageCount: e.messageCount,
  };
}

/** Whether Attention queue should include this entity. */
export function entityNeedsAttention(e: SessionEntity): boolean {
  if (e.parentId) return false;
  return (
    e.status === "needs_approval" ||
    e.status === "failed" ||
    e.status === "suspended" ||
    e.hasPendingApproval
  );
}

/**
 * Σ Entity membership for one conversation — sole formula for rail/board
 * runningCount / approvalCount (not ±1 patches, not stale daemon DTO alone).
 */
export function conversationAggregatesFromEntities(
  sessionsById: Record<string, SessionEntity>,
  conversationId: string,
): { runningCount: number; approvalCount: number } {
  const id = conversationId.trim();
  if (!id) return { runningCount: 0, approvalCount: 0 };
  let runningCount = 0;
  let approvalCount = 0;
  for (const e of Object.values(sessionsById)) {
    if (e.conversationId !== id) continue;
    if (entityCountsAsRunning(e)) runningCount += 1;
    if (entityCountsAsApproval(e)) approvalCount += 1;
  }
  return { runningCount, approvalCount };
}
