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
