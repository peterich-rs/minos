/**
 * Hub try_agent_dispatch only auto-routes bare text for single-agent groups.
 * When the workbench resolves a default agent (no @ in the body), prefix
 * `@runtime` so multi-agent rooms still dispatch + TurnCompletionProjector runs.
 * Idempotent if the body already mentions the agent.
 */
export function hubDispatchText(
  messageBody: string,
  agent: string | null | undefined,
  alreadyRouted: boolean,
): string {
  const body = messageBody ?? "";
  if (alreadyRouted || !agent?.trim()) return body;
  const token = agent.trim();
  const lower = body.toLowerCase();
  const already =
    lower.includes(`@${token.toLowerCase()}`) ||
    lower.includes(`@${token}`);
  if (already) return body;
  return `@${token} ${body}`;
}
