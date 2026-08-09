/**
 * Durable Desktop → Hub/daemon Outbox.
 *
 * Persistence: Tauri SQLite (`im_outbox.sqlite3` under app data dir).
 * In-memory mirror is the working set; every mutation fail-closed persists
 * before returning. Tests inject a memory backend via `useMemoryOutboxForTests`.
 *
 * Status machine:
 *   pending ──flush──► inflight ──2xx──► acked
 *                │              └──err──► pending (backoff) | failed_terminal
 *   startup / stale_inflight_ttl ──► pending
 *
 * kinds: user_message | agent_result | reaction_toggle | approval_resolve
 *
 * Identity:
 * - Logical op id = `clientMessageId`
 * - Storage row `id` is `outbox:${clientMessageId}` (local only)
 */

/** Inline to keep this module free of path-alias imports (node:test friendly). */
function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window || "__TAURI__" in window)
  );
}

async function tauriInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

const LEGACY_STORAGE_KEY = "minos.im.outbox.v1";
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
  agentId?: string | null;
  agentSessionId?: string | null;
  clientSentAtMs?: number | null;
  messageSource?: ImOutboxMessageSource;
  status: OutboxStatus;
  attempts: number;
  nextAttemptAt: number;
  lastError?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
};

/** Wire DTO for Tauri (camelCase matches serde rename_all). */
type ImOutboxWire = {
  id: string;
  kind: string;
  conversationId: string;
  clientMessageId: string;
  text: string;
  title?: string | null;
  replyToMessageId?: string | null;
  agentRuntimes?: string[] | null;
  agentId?: string | null;
  agentSessionId?: string | null;
  clientSentAtMs?: number | null;
  messageSource?: string | null;
  status: string;
  attempts: number;
  nextAttemptAt: number;
  lastError?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
};

type OutboxBackend = {
  load(): Promise<ImOutboxEntry[]>;
  /** Full snapshot replace (boot migrate / memory tests). */
  save(entries: ImOutboxEntry[]): Promise<void>;
  /** Row-level upsert when available (Tauri SQLite). */
  upsert?(entry: ImOutboxEntry): Promise<void>;
};

/**
 * Intent send lane key: same conversation + same intent class = strict FIFO.
 * Different classes (message / reaction / approval) never block each other.
 */
export function outboxLaneKey(
  entry: Pick<ImOutboxEntry, "kind" | "conversationId">,
): string {
  switch (entry.kind) {
    case "user_message":
    case "agent_result":
      return `message:${entry.conversationId}`;
    case "reaction_toggle":
      return `reaction:${entry.conversationId}`;
    case "approval_resolve":
      return `approval:${entry.conversationId}`;
    default: {
      const _exhaustive: never = entry.kind;
      return `unknown:${_exhaustive}`;
    }
  }
}

function nowMs(): number {
  return Date.now();
}

function entryToWire(e: ImOutboxEntry): ImOutboxWire {
  return {
    id: e.id,
    kind: e.kind,
    conversationId: e.conversationId,
    clientMessageId: e.clientMessageId,
    text: e.text,
    title: e.title ?? null,
    replyToMessageId: e.replyToMessageId ?? null,
    agentRuntimes: e.agentRuntimes ?? null,
    agentId: e.agentId ?? null,
    agentSessionId: e.agentSessionId ?? null,
    clientSentAtMs: e.clientSentAtMs ?? null,
    messageSource: e.messageSource ?? null,
    status: e.status,
    attempts: e.attempts,
    nextAttemptAt: e.nextAttemptAt,
    lastError: e.lastError ?? null,
    createdAtMs: e.createdAtMs,
    updatedAtMs: e.updatedAtMs,
  };
}

function wireToEntry(w: ImOutboxWire): ImOutboxEntry {
  return {
    id: w.id,
    kind: w.kind as OutboxKind,
    conversationId: w.conversationId,
    clientMessageId: w.clientMessageId,
    text: w.text,
    title: w.title,
    replyToMessageId: w.replyToMessageId,
    agentRuntimes: w.agentRuntimes ?? undefined,
    agentId: w.agentId,
    agentSessionId: w.agentSessionId,
    clientSentAtMs: w.clientSentAtMs,
    messageSource: (w.messageSource as ImOutboxMessageSource | null) ?? undefined,
    status: w.status as OutboxStatus,
    attempts: w.attempts,
    nextAttemptAt: w.nextAttemptAt,
    lastError: w.lastError,
    createdAtMs: w.createdAtMs,
    updatedAtMs: w.updatedAtMs,
  };
}

function memoryBackend(seed: ImOutboxEntry[] = []): OutboxBackend {
  let rows = seed.slice();
  return {
    async load() {
      return rows.slice();
    },
    async save(entries) {
      rows = entries.slice();
    },
    async upsert(entry) {
      const idx = rows.findIndex((e) => e.clientMessageId === entry.clientMessageId);
      if (idx >= 0) rows[idx] = entry;
      else rows.push(entry);
    },
  };
}

function tauriSqliteBackend(): OutboxBackend {
  return {
    async load() {
      const rows = await tauriInvoke<ImOutboxWire[]>("im_outbox_list_all");
      return (rows ?? []).map(wireToEntry);
    },
    async save(entries) {
      await tauriInvoke("im_outbox_replace_all", {
        entries: entries.map(entryToWire),
      });
    },
    async upsert(entry) {
      await tauriInvoke("im_outbox_upsert", { entry: entryToWire(entry) });
    },
  };
}

/** One-time import of legacy localStorage v1 into SQLite. */
function readLegacyLocalStorage(): ImOutboxEntry[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(LEGACY_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as { entries?: ImOutboxEntry[] };
    if (!parsed || !Array.isArray(parsed.entries)) return [];
    return parsed.entries;
  } catch {
    return [];
  }
}

function clearLegacyLocalStorage(): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.removeItem(LEGACY_STORAGE_KEY);
  } catch {
    /* ignore */
  }
}

// ── Module state ──────────────────────────────────────────────────────────

let backend: OutboxBackend = memoryBackend();
let entries: ImOutboxEntry[] = [];
let ready: Promise<void> | null = null;
let useMemoryOnly = false;
/** Serialize mutate + persist so concurrent enqueue/mark cannot lose rows. */
let mutationChain: Promise<void> = Promise.resolve();

async function ensureReady(): Promise<void> {
  if (!ready) {
    ready = (async () => {
      if (!useMemoryOnly && isTauriRuntime()) {
        backend = tauriSqliteBackend();
      } else {
        backend = memoryBackend();
      }
      entries = await backend.load();
      // Migrate legacy localStorage once when SQLite is empty.
      if (
        !useMemoryOnly &&
        isTauriRuntime() &&
        entries.length === 0
      ) {
        const legacy = readLegacyLocalStorage();
        if (legacy.length > 0) {
          entries = legacy;
          await backend.save(entries);
          clearLegacyLocalStorage();
        }
      }
    })();
  }
  await ready;
}

async function withMutation<T>(fn: () => Promise<T>): Promise<T> {
  await ensureReady();
  let result!: T;
  const run = mutationChain.then(async () => {
    result = await fn();
  });
  mutationChain = run.then(
    () => undefined,
    () => undefined,
  );
  await run;
  return result;
}

async function persistEntry(entry: ImOutboxEntry): Promise<void> {
  try {
    if (backend.upsert) {
      await backend.upsert(entry);
    } else {
      await backend.save(entries);
    }
  } catch (error) {
    console.error("[im-outbox] durable persist failed", error);
    throw error instanceof Error
      ? error
      : new Error(`im-outbox persist failed: ${String(error)}`);
  }
}

async function persistAll(): Promise<void> {
  try {
    await backend.save(entries);
  } catch (error) {
    console.error("[im-outbox] durable persist failed", error);
    throw error instanceof Error
      ? error
      : new Error(`im-outbox persist failed: ${String(error)}`);
  }
}

/** App bootstrap: load SQLite + migrate localStorage. Safe to call multiple times. */
export async function initImOutbox(): Promise<void> {
  await ensureReady();
}

/** Test helper: pure memory backend (no Tauri). */
export function useMemoryOutboxForTests(): void {
  useMemoryOnly = true;
  ready = null;
  backend = memoryBackend();
  entries = [];
  mutationChain = Promise.resolve();
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

async function upsertPendingEntry(input: {
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
}): Promise<ImOutboxEntry> {
  return withMutation(async () => {
    const clientMessageId = input.clientMessageId.trim();
    const conversationId = input.conversationId.trim();
    const text = input.text.trim();
    const t = nowMs();
    const existingIdx = entries.findIndex(
      (e) => e.clientMessageId === clientMessageId,
    );
    if (existingIdx >= 0) {
      const prev = entries[existingIdx]!;
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
      entries[existingIdx] = next;
      await persistEntry(next);
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
    entries.push(entry);
    await persistEntry(entry);
    return entry;
  });
}

export async function enqueueUserMessage(input: {
  conversationId: string;
  clientMessageId: string;
  text: string;
  title?: string | null;
  replyToMessageId?: string | null;
  agentRuntimes?: Array<string | null | undefined>;
  clientSentAtMs?: number | null;
  messageSource?: ImOutboxMessageSource;
}): Promise<ImOutboxEntry> {
  return upsertPendingEntry({
    kind: "user_message",
    ...input,
  });
}

export async function enqueueAgentResult(input: {
  conversationId: string;
  clientMessageId: string;
  text: string;
  agentId?: string | null;
  agentSessionId?: string | null;
  agentRuntimes?: Array<string | null | undefined>;
  replyToMessageId?: string | null;
}): Promise<ImOutboxEntry> {
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

export async function enqueueReactionToggle(input: {
  conversationId: string;
  clientMessageId: string;
  text: string;
}): Promise<ImOutboxEntry> {
  return upsertPendingEntry({
    kind: "reaction_toggle",
    conversationId: input.conversationId,
    clientMessageId: input.clientMessageId,
    text: input.text,
    messageSource: "client_live",
  });
}

export async function enqueueApprovalResolve(input: {
  conversationId: string;
  clientMessageId: string;
  text: string;
}): Promise<ImOutboxEntry> {
  return upsertPendingEntry({
    kind: "approval_resolve",
    conversationId: input.conversationId,
    clientMessageId: input.clientMessageId,
    text: input.text,
    messageSource: "client_live",
  });
}

export async function isAcked(clientMessageId: string): Promise<boolean> {
  await ensureReady();
  const id = clientMessageId.trim();
  if (!id) return false;
  return entries.some((e) => e.clientMessageId === id && e.status === "acked");
}

export async function getOutboxEntry(
  clientMessageId: string,
): Promise<ImOutboxEntry | null> {
  await ensureReady();
  const id = clientMessageId.trim();
  if (!id) return null;
  return entries.find((e) => e.clientMessageId === id) ?? null;
}

export async function markAcked(clientMessageId: string): Promise<void> {
  await withMutation(async () => {
    const id = clientMessageId.trim();
    if (!id) return;
    const t = nowMs();
    for (const e of entries) {
      if (e.clientMessageId === id && e.status !== "acked") {
        e.status = "acked";
        e.updatedAtMs = t;
        e.lastError = null;
        await persistEntry(e);
        return;
      }
    }
  });
}

export async function markInflight(clientMessageId: string): Promise<void> {
  await withMutation(async () => {
    const id = clientMessageId.trim();
    if (!id) return;
    const t = nowMs();
    for (const e of entries) {
      if (e.clientMessageId === id && e.status !== "acked") {
        e.status = "inflight";
        e.attempts += 1;
        e.updatedAtMs = t;
        await persistEntry(e);
        return;
      }
    }
  });
}

export async function markFailed(
  clientMessageId: string,
  error: string,
): Promise<OutboxStatus> {
  return withMutation(async () => {
    const id = clientMessageId.trim();
    const t = nowMs();
    const failureClass = classifyOutboxFailure(error);
    let status: OutboxStatus = "pending";
    for (const e of entries) {
      if (e.clientMessageId !== id) continue;
      if (e.status === "acked") {
        status = "acked";
        continue;
      }
      e.lastError = error;
      e.updatedAtMs = t;
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
      await persistEntry(e);
      return status;
    }
    return status;
  });
}

export async function reclaimStaleInflight(now = nowMs()): Promise<number> {
  return withMutation(async () => {
    const cutoff = now - STALE_INFLIGHT_MS;
    let n = 0;
    const changed: ImOutboxEntry[] = [];
    for (const e of entries) {
      if (e.status !== "inflight") continue;
      if (e.updatedAtMs >= cutoff) continue;
      e.status = "pending";
      e.nextAttemptAt = now;
      e.updatedAtMs = now;
      e.lastError = e.lastError ?? "stale_inflight_reclaimed";
      changed.push(e);
      n += 1;
    }
    for (const e of changed) {
      await persistEntry(e);
    }
    return n;
  });
}

export async function listDuePending(now = nowMs()): Promise<ImOutboxEntry[]> {
  await reclaimStaleInflight(now);
  return entries.filter(
    (e) => e.status === "pending" && e.nextAttemptAt <= now,
  );
}

export function compareOutboxFifo(a: ImOutboxEntry, b: ImOutboxEntry): number {
  if (a.createdAtMs !== b.createdAtMs) return a.createdAtMs - b.createdAtMs;
  return a.clientMessageId.localeCompare(b.clientMessageId);
}

/**
 * Per-intent-lane rows whose head is currently due.
 * Lane key = message|reaction|approval × conversation (see `outboxLaneKey`).
 * Head failure / inflight blocks only that lane's tail.
 */
export async function listDuePendingLanes(
  now = nowMs(),
): Promise<ImOutboxEntry[][]> {
  await reclaimStaleInflight(now);
  const byLane = new Map<string, ImOutboxEntry[]>();
  for (const e of entries) {
    if (e.status !== "pending" && e.status !== "inflight") continue;
    const key = outboxLaneKey(e);
    const list = byLane.get(key);
    if (list) list.push(e);
    else byLane.set(key, [e]);
  }

  const lanes: ImOutboxEntry[][] = [];
  const keys = [...byLane.keys()].sort((a, b) => a.localeCompare(b));
  for (const key of keys) {
    const laneEntries = byLane.get(key)!;
    laneEntries.sort(compareOutboxFifo);
    const head = laneEntries[0];
    if (!head) continue;
    if (head.status !== "pending" || head.nextAttemptAt > now) continue;
    const lane: ImOutboxEntry[] = [];
    for (const e of laneEntries) {
      if (e.status !== "pending" || e.nextAttemptAt > now) break;
      lane.push(e);
    }
    if (lane.length > 0) lanes.push(lane);
  }
  return lanes;
}

export async function earliestPendingAttemptAt(
  now = nowMs(),
): Promise<number | null> {
  await reclaimStaleInflight(now);
  let min: number | null = null;
  for (const e of entries) {
    if (e.status !== "pending") continue;
    if (min == null || e.nextAttemptAt < min) min = e.nextAttemptAt;
  }
  return min;
}

export async function listUnsynced(): Promise<ImOutboxEntry[]> {
  await ensureReady();
  return entries.filter(
    (e) =>
      e.status === "pending" ||
      e.status === "inflight" ||
      e.status === "failed_terminal",
  );
}

export async function listPendingForConversation(
  conversationId: string,
): Promise<ImOutboxEntry[]> {
  await ensureReady();
  const cid = conversationId.trim();
  return entries.filter(
    (e) =>
      e.conversationId === cid &&
      (e.status === "pending" || e.status === "inflight"),
  );
}

export async function resetImOutboxForTests(): Promise<void> {
  useMemoryOutboxForTests();
  await withMutation(async () => {
    entries = [];
    await persistAll();
  });
}

export async function getOutboxSnapshotForTests(): Promise<ImOutboxEntry[]> {
  await ensureReady();
  return entries.slice();
}

export async function forceUpdatedAtForTests(
  clientMessageId: string,
  updatedAtMs: number,
): Promise<void> {
  await withMutation(async () => {
    const id = clientMessageId.trim();
    for (const e of entries) {
      if (e.clientMessageId === id) {
        e.updatedAtMs = updatedAtMs;
        await persistEntry(e);
        return;
      }
    }
  });
}
