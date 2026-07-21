/** Parse `@agent` / `@agent#short` routing (aligned with minos-tui agent_route.rs). */

export const KNOWN_AGENTS = [
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
] as const;

export type KnownAgent = (typeof KNOWN_AGENTS)[number];

export type AgentRouteTarget = {
  agent: KnownAgent;
  threadShortId?: string;
};

/** First up-to-8 chars of a thread id (TUI `short_thread_id` parity). */
export function shortThreadId(threadId: string): string {
  let end = Math.min(8, threadId.length);
  // Avoid splitting multi-byte code units at the cut (rare for hex ids).
  while (end > 0 && (threadId.charCodeAt(end - 1) & 0xfc00) === 0xdc00) {
    end -= 1;
  }
  return threadId.slice(0, end);
}

export function parseAgentName(value: string): KnownAgent | null {
  const normalized = value.toLowerCase();
  return (KNOWN_AGENTS as readonly string[]).includes(normalized)
    ? (normalized as KnownAgent)
    : null;
}

export function parseAgentRouteTarget(value: string): AgentRouteTarget | null {
  const [agentPart, shortPart] = value.split("#");
  if (shortPart !== undefined && shortPart.length === 0) return null;
  const agent = parseAgentName(agentPart ?? "");
  if (!agent) return null;
  return {
    agent,
    threadShortId: shortPart || undefined,
  };
}

/**
 * `@codex hello` → { target: { agent: "codex" }, prompt: "hello", messageBody: "@codex hello" }
 * plain text → null (caller may fall back to default agent)
 *
 * Routing semantics (TUI parity):
 * - `@agent prompt` → start a **new** session for that agent
 * - `@agent#short prompt` → continue an existing session
 */
export function parseAgentRouting(
  text: string,
): { target: AgentRouteTarget; prompt: string; messageBody: string } | null {
  const messageBody = text.trimEnd();
  const trimmed = text.trimStart();
  if (!trimmed.startsWith("@")) return null;
  const rest = trimmed.slice(1);
  const splitAt = [...rest].findIndex((ch) => /\s/.test(ch));
  const token = splitAt === -1 ? rest : rest.slice(0, splitAt);
  const body = splitAt === -1 ? "" : rest.slice(splitAt).trimStart();
  const target = parseAgentRouteTarget(token);
  if (!target) return null;
  return { target, prompt: body, messageBody };
}

/** Active @-token at cursor for autocomplete (TUI-style). */
export function mentionQueryAtCursor(
  text: string,
  cursor: number,
): { start: number; query: string } | null {
  const before = text.slice(0, cursor);
  const at = before.lastIndexOf("@");
  if (at < 0) return null;
  if (at > 0 && !/\s/.test(before[at - 1] ?? " ")) return null;
  const token = before.slice(at + 1);
  if (/\s/.test(token)) return null;
  return { start: at, query: token };
}

export type MentionOption = {
  id: string;
  label: string;
  hint: string;
  insert: string;
  disabled: boolean;
};

export type MentionCli = {
  agent: string;
  installed: boolean;
  status: string;
};

export type MentionSession = {
  id: string;
  agent: string;
  shortId: string;
  status: string;
  parentId?: string | null;
};

/**
 * TUI-parity @-picker rows:
 * 1. Installed (or known) agents as bare `@agent` → start a new session
 * 2. Existing open sessions as `@agent#short` → continue that run
 */
export function buildAgentMentionOptions(
  query: string,
  clis: readonly MentionCli[],
  sessions: readonly MentionSession[],
  limit = 16,
): MentionOption[] {
  const q = query.toLowerCase();
  const matches = (s: string) => !q || s.toLowerCase().includes(q);

  const fromCli = clis
    .filter((c) => matches(c.agent) || matches(`@${c.agent}`))
    .map((c) => ({
      id: `new:${c.agent}`,
      label: `@${c.agent}`,
      hint: c.installed ? "new session" : "not installed",
      insert: `@${c.agent} `,
      disabled: !c.installed,
    }));

  const fromKnown =
    fromCli.length > 0
      ? fromCli
      : KNOWN_AGENTS.filter((a) => matches(a) || matches(`@${a}`)).map(
          (a) => ({
            id: `new:${a}`,
            label: `@${a}`,
            hint: "new session",
            insert: `@${a} `,
            disabled: false,
          }),
        );

  const fromSessions = sessions
    .filter((s) => !s.parentId)
    .filter((s) => s.status !== "done" && s.status !== "failed")
    .filter(
      (s) =>
        matches(s.agent) ||
        matches(s.shortId) ||
        matches(`@${s.agent}#${s.shortId}`),
    )
    .map((s) => ({
      id: `sess:${s.id}`,
      label: `@${s.agent}#${s.shortId}`,
      hint: `continue · ${s.status}`,
      insert: `@${s.agent}#${s.shortId} `,
      disabled: false,
    }));

  return [...fromKnown, ...fromSessions].slice(0, limit);
}
