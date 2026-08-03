/**
 * Durable Desktop → Hub/daemon Outbox (localStorage).
 *
 * Status machine:
 *   pending ──flush──► inflight ──2xx──► acked
 *                │              └──err──► pending (backoff) | failed_terminal
 *   startup / stale_inflight_ttl ──► pending
 *
 * kinds: user_message | agent_result | reaction_toggle | approval_resolve
 * (same status machine)
 *
 * Identity:
 * - Logical op id = `clientMessageId` (B6 reaction `client_op_id`, C5.3
 *   approval `client_request_id`, user message `client_message_id`).
 * - Storage row `id` is `outbox:${clientMessageId}` (local only; never on wire).
 */

const STORAGE_KEY = "minos.im.outbox.v1";
const MAX_ENTRIES = 500;
/** Permanent (business/client) errors may terminal after this many attempts. */
const MAX_PERMANENT_ATTEMPTS = 8;
const BASE_BACKOFF_MS = 1_500;
/** Cap backoff so long offline keeps retrying without terminal. */
const MAX_BACKOFF_MS = 5 * 60_000;
/** Inflight rows older than this are reclaimed to pending (kill mid-flight). */
export const STALE_INFLIGHT_MS = 45_000;

export type OutboxStatus =
  | "pending"
  | "inflight"
  | "acked"
  | "failed_terminal";

export type OutboxKind =
  | "user_message"
  | "agent_result"
  | "reaction_toggle"
  | "approval_resolve";

export type ImOutboxMessageSource =
  | "client_live"
  | "host_projection"
  | "system";

export type ImOutboxEntry = {
  id: string;
  kind: OutboxKind;
  conversationId: string;
  clientMessageId: string;
  text: string;
  title?: string | null;
  replyToMessageId?: string | null;
  agentRuntimes?: string[];
  /** Cloud agent_id for agent_result posts (resolved at enqueue time when known). */
  agentId?: string | null;
  agentSessionId?: string | null;
  clientSentAtMs?: number | null;
  /** Hub write provenance; Linked Hub-first defaults to client_live. */
  messageSource?: ImOutboxMessageSource;
  status: OutboxStatus;
  attempts: number;
  nextAttemptAt: number;
  lastError?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
};

type OutboxStore = {
  entries: ImOutboxEntry[];
};

function nowMs(): number {
  return Date.now();
}

function loadStore(): OutboxStore {
  if (typeof localStorage === "undefined") {
    return { entries: [] };
  }
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { entries: [] };
    const parsed = JSON.parse(raw) as OutboxStore;
    if (!parsed || !Array.isArray(parsed.entries)) return { entries: [] };
    return { entries: parsed.entries };
  } catch {
    return { entries: [] };
  }
}

function saveStore(store: OutboxStore): void {
  if (typeof localStorage === "undefined") return;
  // Cap size: drop oldest acked first, then oldest overall.
  let entries = store.entries.slice();
  if (entries.length > MAX_ENTRIES) {
    const acked = entries.filter((e) => e.status === "acked");
    const rest = entries.filter((e) => e.status !== "acked");
    acked.sort((a, b) => a.updatedAtMs - b.updatedAtMs);
    while (acked.length + rest.length > MAX_ENTRIES && acked.length > 0) {
      acked.shift();
    }
    entries = [...rest, ...acked].sort((a, b) => a.createdAtMs - b.createdAtMs);
    if (entries.length > MAX_ENTRIES) {
      entries = entries.slice(entries.length - MAX_ENTRIES);
    }
  }
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ entries }));
  } catch (error) {
    console.warn("[im-outbox] failed to persist", error);
  }
}

function backoffMs(attempts: number): number {
  const exp = Math.min(attempts, 6);
  return Math.min(BASE_BACKOFF_MS * 2 ** exp, MAX_BACKOFF_MS);
}

/** Permanent client/business errors may exhaust; network stays pending forever. */
export type OutboxFailureClass = "transient" | "permanent";

export function classifyOutboxFailure(error: string): OutboxFailureClass {
  const e = error.toLowerCase();
  if (
    e.includes("invalid_payload") ||
    e.includes("empty_text") ||
    e.includes("invalid payload") ||
    e.includes("permission") ||
    e.includes("forbidden") ||
    e.includes("unauthorized") ||
    e.includes("not found") ||
    e.includes("http 4") ||
    e.includes("status: 4") ||
    /\b4\d\d\b/.test(e)
  ) {
    if (
      e.includes("408") ||
      e.includes("429") ||
      e.includes("timeout") ||
      e.includes("too many")
    ) {
      return "transient";
    }
    return "permanent";
  }
  return "transient";
}

function upsertPendingEntry(
  input: {
    kind: OutboxKind;
    conversationId: string;
    clientMessageId: string;
    text: string;
    title?: string | null;
    replyToMessageId?: string | null;
    agentRuntimes?: Array<string | null | undefined>;
    agentId?: string | null;
    agentSessionId?: string | null;
    clientSentAtMs?: number | null;
    messageSource?: ImOutboxMessageSource;
  },
): ImOutboxEntry {
  const store = loadStore();
  const clientMessageId = input.clientMessageId.trim();
  const conversationId = input.conversationId.trim();
  const text = input.text.trim();
  const t = nowMs();
  const existingIdx = store.entries.findIndex(
    (e) => e.clientMessageId === clientMessageId,
  );
  if (existingIdx >= 0) {
    const prev = store.entries[existingIdx]!;
    if (prev.status === "acked") {
      return prev;
    }
    const next: ImOutboxEntry = {
      ...prev,
      kind: input.kind,
      text,
      title: input.title ?? prev.title,
      replyToMessageId: input.replyToMessageId ?? prev.replyToMessageId,
      agentRuntimes:
        input.agentRuntimes
          ?.map((r) => r?.trim())
          .filter((r): r is string => !!r) ?? prev.agentRuntimes,
      agentId: input.agentId ?? prev.agentId,
      agentSessionId: input.agentSessionId ?? prev.agentSessionId,
      clientSentAtMs: input.clientSentAtMs ?? prev.clientSentAtMs,
      messageSource: input.messageSource ?? prev.messageSource,
      status: "pending",
      nextAttemptAt: t,
      updatedAtMs: t,
      lastError: null,
    };
    store.entries[existingIdx] = next;
    saveStore(store);
    return next;
  }

  const entry: ImOutboxEntry = {
    id: `outbox:${clientMessageId}`,
    kind: input.kind,
    conversationId,
    clientMessageId,
    text,
    title: input.title,
    replyToMessageId: input.replyToMessageId,
    agentRuntimes: input.agentRuntimes
      ?.map((r) => r?.trim())
      .filter((r): r is string => !!r),
    agentId: input.agentId,
    agentSessionId: input.agentSessionId,
    clientSentAtMs: input.clientSentAtMs,
    messageSource: input.messageSource ?? "client_live",
    status: "pending",
    attempts: 0,
    nextAttemptAt: t,
    lastError: null,
    createdAtMs: t,
    updatedAtMs: t,
  };
  store.entries.push(entry);
  saveStore(store);
  return entry;
}

/** Enqueue (or refresh) a user message for Hub projection. */
export function enqueueUserMessage(input: {
  conversationId: string;
  clientMessageId: string;
  text: string;
  title?: string | null;
  replyToMessageId?: string | null;
  agentRuntimes?: Array<string | null | undefined>;
  clientSentAtMs?: number | null;
  messageSource?: ImOutboxMessageSource;
}): ImOutboxEntry {
  return upsertPendingEntry({
    kind: "user_message",
    ...input,
  });
}

/** Enqueue (or refresh) an agent final-bubble host projection. */
export function enqueueAgentResult(input: {
  conversationId: string;
  clientMessageId: string;
  text: string;
  agentId?: string | null;
  agentSessionId?: string | null;
  agentRuntimes?: Array<string | null | undefined>;
  replyToMessageId?: string | null;
}): ImOutboxEntry {
  return upsertPendingEntry({
    kind: "agent_result",
    conversationId: input.conversationId,
    clientMessageId: input.clientMessageId,
    text: input.text,
    agentId: input.agentId,
    agentSessionId: input.agentSessionId,
    agentRuntimes: input.agentRuntimes,
    replyToMessageId: input.replyToMessageId,
    messageSource: "host_projection",
  });
}

/**
 * Enqueue reaction toggle. `clientMessageId` is the B6 `client_op_id`
 * (event_id suffix) and must stay stable across retries.
 */
export function enqueueReactionToggle(input: {
  conversationId: string;
  clientMessageId: string;
  /** JSON: `{ messageId, emoji }`. */
  text: string;
}): ImOutboxEntry {
  return upsertPendingEntry({
    kind: "reaction_toggle",
    conversationId: input.conversationId,
    clientMessageId: input.clientMessageId,
    text: input.text,
    messageSource: "client_live",
  });
}

/**
 * Enqueue approval resolve for durable retry when daemon is briefly
 * unreachable (C5.3). Payload JSON: `{ requestId, sessionId, decision }`.
 */
export function enqueueApprovalResolve(input: {
  conversationId: string;
  clientMessageId: string;
  text: string;
}): ImOutboxEntry {
  return upsertPendingEntry({
    kind: "approval_resolve",
    conversationId: input.conversationId,
    clientMessageId: input.clientMessageId,
    text: input.text,
    messageSource: "client_live",
  });
}

export function isAcked(clientMessageId: string): boolean {
  const id = clientMessageId.trim();
  if (!id) return false;
  return loadStore().entries.some(
    (e) => e.clientMessageId === id && e.status === "acked",
  );
}

export function markAcked(clientMessageId: string): void {
  const id = clientMessageId.trim();
  if (!id) return;
  const store = loadStore();
  const t = nowMs();
  let changed = false;
  for (const e of store.entries) {
    if (e.clientMessageId === id && e.status !== "acked") {
      e.status = "acked";
      e.updatedAtMs = t;
      e.lastError = null;
      changed = true;
    }
  }
  if (changed) saveStore(store);
}

export function markInflight(clientMessageId: string): void {
  const id = clientMessageId.trim();
  if (!id) return;
  const store = loadStore();
  const t = nowMs();
  let changed = false;
  for (const e of store.entries) {
    if (e.clientMessageId === id && e.status !== "acked") {
      e.status = "inflight";
      e.attempts += 1;
      e.updatedAtMs = t;
      changed = true;
    }
  }
  if (changed) saveStore(store);
}

export function markFailed(
  clientMessageId: string,
  error: string,
): OutboxStatus {
  const id = clientMessageId.trim();
  const store = loadStore();
  const t = nowMs();
  const failureClass = classifyOutboxFailure(error);
  let status: OutboxStatus = "pending";
  for (const e of store.entries) {
    if (e.clientMessageId !== id) continue;
    if (e.status === "acked") {
      status = "acked";
      continue;
    }
    e.lastError = error;
    e.updatedAtMs = t;
    // Transient network: never burn to failed_terminal — long offline must
    // still deliver once after reconnect (capped backoff only).
    if (
      failureClass === "permanent" &&
      e.attempts >= MAX_PERMANENT_ATTEMPTS
    ) {
      e.status = "failed_terminal";
      status = "failed_terminal";
    } else {
      e.status = "pending";
      e.nextAttemptAt = t + backoffMs(e.attempts);
      status = "pending";
    }
  }
  saveStore(store);
  return status;
}

/**
 * Reclaim inflight rows whose updatedAt is older than STALE_INFLIGHT_MS.
 * Returns the number of rows reclaimed to pending.
 */
export function reclaimStaleInflight(now = nowMs()): number {
  const store = loadStore();
  const cutoff = now - STALE_INFLIGHT_MS;
  let n = 0;
  for (const e of store.entries) {
    if (e.status !== "inflight") continue;
    if (e.updatedAtMs >= cutoff) continue;
    e.status = "pending";
    e.nextAttemptAt = now;
    e.updatedAtMs = now;
    e.lastError = e.lastError ?? "stale_inflight_reclaimed";
    n += 1;
  }
  if (n > 0) saveStore(store);
  return n;
}

/**
 * Pending due now, plus stale inflight (reclaimed inline so kill mid-flight
 * does not leave a permanent black hole).
 *
 * All kinds (including reaction_toggle / approval_resolve) are drainable.
 */
export function listDuePending(now = nowMs()): ImOutboxEntry[] {
  reclaimStaleInflight(now);
  return loadStore().entries.filter(
    (e) => e.status === "pending" && e.nextAttemptAt <= now,
  );
}

export function listUnsynced(): ImOutboxEntry[] {
  return loadStore().entries.filter(
    (e) =>
      e.status === "pending" ||
      e.status === "inflight" ||
      e.status === "failed_terminal",
  );
}

export function listPendingForConversation(
  conversationId: string,
): ImOutboxEntry[] {
  const cid = conversationId.trim();
  return loadStore().entries.filter(
    (e) =>
      e.conversationId === cid &&
      (e.status === "pending" || e.status === "inflight"),
  );
}

/** Test / account-switch helper. */
export function resetImOutboxForTests(): void {
  if (typeof localStorage === "undefined") return;
  localStorage.removeItem(STORAGE_KEY);
}

export function getOutboxSnapshotForTests(): ImOutboxEntry[] {
  return loadStore().entries.slice();
}

/** Test helper: force updatedAtMs on an entry (simulate kill mid-inflight). */
export function forceUpdatedAtForTests(
  clientMessageId: string,
  updatedAtMs: number,
): void {
  const store = loadStore();
  const id = clientMessageId.trim();
  for (const e of store.entries) {
    if (e.clientMessageId === id) {
      e.updatedAtMs = updatedAtMs;
    }
  }
  saveStore(store);
}
