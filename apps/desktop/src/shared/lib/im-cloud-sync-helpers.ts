/**
 * Pure helpers for Desktop → Hub IM dual-write (no account / network deps).
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

/** Whether a timeline row should dual-write as an agent social message. */
export function isProjectableAgentMessage(m: {
  id: string;
  role: string;
  body?: string | null;
}): boolean {
  if (!m.id?.trim() || !m.body?.trim()) return false;
  return m.role === "agent" || m.id.startsWith("agent-result:");
}
