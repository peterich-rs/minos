/**
 * Pure participant-delivery target resolution (no daemon / store deps).
 * Room rules: ADR 0021 / agent-participant-delivery.
 */

import {
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

  // Bare text: sole agent member may auto-route (workbench + Hub group parity).
  if (members.length === 1) {
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
