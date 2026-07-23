/**
 * L4 SessionEntity — process-wide light truth for session status /
 * hasPendingApproval. All UI status mutations must go through merge helpers
 * here (or store wrappers that call them); list caches only project Entity.
 *
 * hasPendingApproval is the fallback truth when Transcript is missing/evicted:
 * never demote needs_approval by scanning a missing transcript window.
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

export type SessionEntitySeed = {
  id: string;
  conversationId: string;
  conversationTitle?: string;
  agent: string;
  shortId: string;
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
  /** Override updatedAtMs (tests). */
  nowMs?: number;
};

/**
 * Merge a daemon/list seed + optional pending signal into SessionEntity.
 * Sole pure writer for status/hasPendingApproval derivation.
 */
export function mergeSessionEntity(
  prev: SessionEntity | undefined,
  seed: SessionEntitySeed,
  opts: MergeSessionEntityOptions = {},
): SessionEntity {
  const daemonStatus = asDaemonStatus(seed.status);
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
    conversationTitle:
      seed.conversationTitle ?? prev?.conversationTitle,
    agent: seed.agent || prev?.agent || "codex",
    shortId: seed.shortId || prev?.shortId || seed.id.slice(0, 8),
    daemonStatus,
    model: seed.model || prev?.model || "",
    parentId: seed.parentId ?? prev?.parentId,
    summary: seed.summary ?? prev?.summary ?? "",
    messageCount: seed.messageCount ?? prev?.messageCount ?? 0,
    firstTsMs: seed.firstTsMs ?? prev?.firstTsMs,
    lastTsMs: seed.lastTsMs ?? prev?.lastTsMs,
    needsContinue: seed.needsContinue ?? prev?.needsContinue,
    hasPendingApproval,
    status: entityUiStatus(hasPendingApproval, daemonStatus),
    updatedAtMs: opts.nowMs ?? Date.now(),
  };
}

/**
 * Patch fields on an existing entity (or create a minimal shell).
 * Recomputes status from hasPendingApproval + daemonStatus after merge.
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
