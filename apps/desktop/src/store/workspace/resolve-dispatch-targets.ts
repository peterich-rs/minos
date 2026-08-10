/**
 * Pure participant-delivery target resolution (no daemon / store deps).
 * Room rules: ADR 0021 / agent-participant-delivery.
 */

import {
  KNOWN_AGENTS,
  parseAllAgentRoutings,
  type KnownAgent,
  type MentionProfile,
} from "../../shared/lib/agent-route.ts";

export type DispatchTarget = {
  agent: KnownAgent;
  prompt: string;
  sessionShortId?: string;
  profileId?: string;
};

export type ResolveDispatchTargetsInput = {
  messageBody: string;
  participatingAgents: string[] | undefined;
  installedAgents: ReadonlySet<string>;
  mentionProfiles: MentionProfile[];
  /**
   * Human member count when known (Hub participants). Sole-agent auto-route
   * requires exactly 1 human + 1 agent (group parity with Hub). When omitted,
   * treated as 1 (Desktop workbench default for sole-bot rooms).
   */
  humanMemberCount?: number;
  /**
   * Conversation kind when known. Sole-agent auto-route is group-only on Hub;
   * when omitted, treated as "group" for Desktop workbench rooms.
   */
  conversationKind?: string;
};

/**
 * Resolve agent delivery targets for a user turn.
 * Returns empty targets for pure human / multi-bot bare text (no throw).
 * Throws only for invalid explicit @ (non-member) or sole-bot room with empty prompt.
 */
export function resolveDispatchTargets(
  input: ResolveDispatchTargetsInput,
): { targets: DispatchTarget[]; multiRoutedCount: number } {
  const messageBody = input.messageBody;
  const multiRouted = parseAllAgentRoutings(messageBody, input.mentionProfiles);
  const members = (input.participatingAgents ?? [])
    .map((a) => a.trim().toLowerCase())
    .filter((a) => a.length > 0);
  const memberSet = new Set(members);

  const targets: DispatchTarget[] = [];
  if (multiRouted.length > 0) {
    for (const routed of multiRouted) {
      const agent = routed.target.agent;
      if (!memberSet.has(agent)) {
        throw new Error(
          memberSet.size === 0
            ? `@${agent} is not a member of this conversation. Add the bot as a participant before @mentioning.`
            : `@${agent} is not a member of this conversation. Only roster agents can be @mentioned.`,
        );
      }
      targets.push({
        agent,
        prompt: routed.prompt,
        sessionShortId: routed.target.sessionShortId,
        profileId: routed.target.profileId,
      });
    }
    return { targets, multiRoutedCount: multiRouted.length };
  }

  // Unresolved agentish @ (known runtime name not on roster) blocks sole-route.
  // parseAllAgentRoutings only returns members that resolve as KnownAgent tokens;
  // unmatched known names still appear as body @tokens and must not activate sole bot.
  const unresolvedAgentish = firstUnresolvedAgentishToken(
    messageBody,
    memberSet,
    input.mentionProfiles,
  );
  if (unresolvedAgentish) {
    throw new Error(
      memberSet.size === 0
        ? `@${unresolvedAgentish} is not a member of this conversation. Add the bot as a participant before @mentioning.`
        : `@${unresolvedAgentish} is not a member of this conversation. Only roster agents can be @mentioned.`,
    );
  }

  // Bare text: sole agent member may auto-route (Hub group + 1 human + 1 agent).
  const kind = (input.conversationKind ?? "group").toLowerCase();
  const humans = input.humanMemberCount ?? 1;
  if (kind === "group" && humans === 1 && members.length === 1) {
    const sole = members[0] as KnownAgent;
    if (!input.installedAgents.has(sole)) {
      throw new Error(
        `Bot @${sole} is a conversation member but not installed on this Host. Install the runtime or @mention only when available.`,
      );
    }
    if (!messageBody.trim()) {
      throw new Error("Cannot start an agent session with an empty prompt.");
    }
    targets.push({ agent: sole, prompt: messageBody });
    return { targets, multiRoutedCount: 0 };
  }

  // 0 agents → pure human IM; multi agents without @ → pure human (no fan-out).
  return { targets: [], multiRoutedCount: 0 };
}

/** First known-agentish @token that is not a roster member (membership-first). */
function firstUnresolvedAgentishToken(
  text: string,
  memberSet: ReadonlySet<string>,
  profiles: readonly MentionProfile[],
): string | null {
  const re = /(^|[\s([{"'`])@([^\s@]+)/g;
  let match: RegExpExecArray | null;
  const profileByName = new Map(
    profiles.map((p) => [p.name.trim().toLowerCase(), p]),
  );
  while ((match = re.exec(text)) !== null) {
    const raw = (match[2] ?? "").trim();
    if (!raw) continue;
    const namePart = raw.split("#")[0]?.trim() ?? raw;
    if (!namePart) continue;
    const lower = namePart.toLowerCase();
    const profile = profileByName.get(lower);
    if (profile) {
      const runtime = profile.runtimeAgent.trim().toLowerCase();
      if (runtime && !memberSet.has(runtime)) {
        return namePart;
      }
      continue;
    }
    const known = (KNOWN_AGENTS as readonly string[]).includes(lower);
    const botish = lower.startsWith("bot-");
    if ((known || botish) && !memberSet.has(lower)) {
      return namePart;
    }
  }
  return null;
}
