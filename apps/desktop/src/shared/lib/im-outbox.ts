/**
 * Durable Desktop → Hub user-message Outbox (localStorage).
 *
 * Status: pending | inflight | acked | failed_terminal
 * - Only pending/failed rows are re-projected.
 * - Acked client_message_ids are never re-POSTed.
 * - Phase 1: simple durable store; Phase 2+ may move to daemon table.
 */

const STORAGE_KEY = "minos.im.outbox.v1";
const MAX_ENTRIES = 500;
const MAX_ATTEMPTS = 8;
const BASE_BACKOFF_MS = 1_500;

export type OutboxStatus =
  | "pending"
  | "inflight"
  | "acked"
  | "failed_terminal";

export type ImOutboxMessageSource =
  | "client_live"
  | "host_projection"
  | "system";

export type ImOutboxEntry = {
  id: string;
  kind: "user_message";
  conversationId: string;
  clientMessageId: string;
  text: string;
  title?: string | null;
  replyToMessageId?: string | null;
  agentRuntimes?: string[];
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
  return BASE_BACKOFF_MS * 2 ** exp;
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
      text,
      title: input.title ?? prev.title,
      replyToMessageId: input.replyToMessageId ?? prev.replyToMessageId,
      agentRuntimes:
        input.agentRuntimes
          ?.map((r) => r?.trim())
          .filter((r): r is string => !!r) ?? prev.agentRuntimes,
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
    kind: "user_message",
    conversationId,
    clientMessageId,
    text,
    title: input.title,
    replyToMessageId: input.replyToMessageId,
    agentRuntimes: input.agentRuntimes
      ?.map((r) => r?.trim())
      .filter((r): r is string => !!r),
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
  let status: OutboxStatus = "pending";
  for (const e of store.entries) {
    if (e.clientMessageId !== id) continue;
    if (e.status === "acked") {
      status = "acked";
      continue;
    }
    e.lastError = error;
    e.updatedAtMs = t;
    if (e.attempts >= MAX_ATTEMPTS) {
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

/** Pending / retryable entries due now (or overdue). Never returns terminal failures. */
export function listDuePending(now = nowMs()): ImOutboxEntry[] {
  return loadStore().entries.filter(
    (e) => e.status === "pending" && e.nextAttemptAt <= now,
  );
}

export function listUnsynced(): ImOutboxEntry[] {
  return loadStore().entries.filter(
    (e) => e.status === "pending" || e.status === "inflight" || e.status === "failed_terminal",
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
