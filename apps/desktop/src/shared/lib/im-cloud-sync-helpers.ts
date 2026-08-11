/**
 * Pure helpers for Desktop → Hub IM projection (no account / network deps).
 */

const VALID_RUNTIMES = new Set([
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
]);

/** Normalize local agent bin name; never treat arbitrary strings as cloud ids. */
export function normalizeHostRuntime(
  runtime: string | null | undefined,
): string | null {
  const r = (runtime ?? "").trim().toLowerCase();
  if (!r || !VALID_RUNTIMES.has(r)) return null;
  return r;
}

export function displayNameForRuntime(runtime: string): string {
  return runtime.charAt(0).toUpperCase() + runtime.slice(1);
}

/**
 * Frozen agent-result id shape:
 * `agent-result:{conversationId}:{sessionId}:{originMessageId}`
 *
 * All three segments after the prefix must be non-empty. Origin may itself
 * contain `:` (UUID-like message ids usually do not).
 */
export function isCanonicalAgentResultId(
  messageId: string,
  conversationId?: string | null,
): boolean {
  const id = messageId.trim();
  if (!id.startsWith("agent-result:")) return false;
  const rest = id.slice("agent-result:".length);
  const first = rest.indexOf(":");
  if (first <= 0) return false;
  const second = rest.indexOf(":", first + 1);
  if (second <= first + 1) return false;
  const conv = rest.slice(0, first);
  const session = rest.slice(first + 1, second);
  const origin = rest.slice(second + 1);
  if (!conv || !session || !origin) return false;
  if (conversationId?.trim() && conv !== conversationId.trim()) return false;
  return true;
}

/** Whether a timeline row should dual-write as an agent social message. */
export function isProjectableAgentMessage(m: {
  id: string;
  role: string;
  body?: string | null;
}): boolean {
  if (!m.id?.trim() || !m.body?.trim()) return false;
  return m.role === "agent" || m.id.startsWith("agent-result:");
}
