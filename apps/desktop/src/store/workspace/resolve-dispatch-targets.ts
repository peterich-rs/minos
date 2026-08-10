/**
 * Pure participant-delivery target resolution (no daemon / store deps).
 * Room rules: ADR 0021 / agent-participant-delivery / global-bot-identity.
 *
 * Delivery targets are **roster-only**. Membership SSOT on Conversation is
 * `participatingBots` (botId + name + runtime). Callers should pass
 * `membershipTokensOfBots(conv.participatingBots)` (or Hub participant tokens)
 * into `participatingAgents` — that field is a **flattened membership token
 * list for resolve**, not the Conversation dual-write field.
 *
 * - `participatingAgents` (input): member tokens (botId / bot name / runtime,
 *   lowercased) — never the full Host profile directory.
 * - `mentionProfiles`: only bots already on the roster (Hub participants or
 *   equivalent). Unjoined global profiles must not be included.
 *
 * Bare runtime names and local-only profiles are **not** multi-end identity;
 * they may only route when those tokens appear on the roster.
 */

import {
  KNOWN_AGENTS,
  parseAgentRouteTarget,
  parseAllAgentRoutings,
  type KnownAgent,
  type MentionHuman,
  type MentionProfile,
} from "../../shared/lib/agent-route.ts";

export type DispatchTarget = {
  agent: KnownAgent;
  prompt: string;
  sessionShortId?: string;
  /**
   * Host profile / Hub agent id when the mention resolved to a named bot body.
   * Present only for explicit profile/bot-name routes, not bare runtime.
   */
  profileId?: string;
};

/**
 * Wire `MentionTarget` for Account WS AppendMessage (snake_case tag `kind`).
 * Backend validates membership only; body never invents delivery.
 */
export type WireMentionTarget =
  | {
      kind: "bot";
      bot_id: string;
      start?: number;
      length?: number;
    }
  | {
      kind: "account";
      account_id: string;
      start?: number;
      length?: number;
    };

export type BuildStructuredMentionsOptions = {
  /**
   * Optional human roster for `@minos_id` → account mentions.
   * When omitted, only bot mentions are emitted.
   */
  mentionHumans?: readonly MentionHuman[];
};

export type ResolveDispatchTargetsInput = {
  messageBody: string;
  /**
   * Flattened membership tokens for this resolve call (lowercased by resolver).
   * SSOT on Conversation is `participatingBots`; build via
   * `membershipTokensOfBots(participatingBots)` or Hub participant agent_id /
   * name / runtime. Not a license to invent dual-write `participatingAgents`.
   */
  participatingAgents: string[] | undefined;
  installedAgents: ReadonlySet<string>;
  /**
   * Roster-scoped bot cards for @Name / @p/<id> parse.
   * Must not include unjoined Host profiles when Hub participants are known.
   */
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
  // Only roster-scoped profiles participate in parse (caller contract).
  const rosterProfiles = input.mentionProfiles;
  const multiRouted = parseAllAgentRoutings(messageBody, rosterProfiles);
  const members = (input.participatingAgents ?? [])
    .map((a) => a.trim().toLowerCase())
    .filter((a) => a.length > 0);
  const memberSet = new Set(members);

  const targets: DispatchTarget[] = [];
  if (multiRouted.length > 0) {
    for (const routed of multiRouted) {
      const agent = routed.target.agent;
      const profileId = routed.target.profileId;
      // Membership: runtime token, or named bot id/name when present on roster.
      const memberOk =
        memberSet.has(agent) ||
        (profileId != null &&
          (memberSet.has(profileId.toLowerCase()) ||
            rosterProfiles.some(
              (p) =>
                p.id === profileId &&
                (memberSet.has(p.name.trim().toLowerCase()) ||
                  memberSet.has(p.runtimeAgent.trim().toLowerCase()) ||
                  memberSet.has(p.id.toLowerCase())),
            )));
      if (!memberOk) {
        throw new Error(
          memberSet.size === 0
            ? `@${agent} is not a member of this conversation. Add the bot as a participant before @mentioning.`
            : `@${agent} is not a member of this conversation. Only roster agents can be @mentioned.`,
        );
      }
      // Named profile/bot must itself be roster-scoped (no unjoined profile fan-in).
      if (profileId) {
        const profile = rosterProfiles.find((p) => p.id === profileId);
        if (!profile) {
          throw new Error(
            `Bot profile is not a member of this conversation. Only roster bots can be @mentioned.`,
          );
        }
        const profileTokenOk =
          memberSet.has(profile.id.toLowerCase()) ||
          memberSet.has(profile.name.trim().toLowerCase()) ||
          memberSet.has(profile.runtimeAgent.trim().toLowerCase());
        if (!profileTokenOk) {
          throw new Error(
            `@${profile.name} is not a member of this conversation. Only roster agents can be @mentioned.`,
          );
        }
      }
      targets.push({
        agent,
        prompt: routed.prompt,
        sessionShortId: routed.target.sessionShortId,
        profileId,
      });
    }
    return { targets, multiRoutedCount: multiRouted.length };
  }

  // Unresolved agentish @ (known runtime / botish name not on roster) blocks sole-route.
  const unresolvedAgentish = firstUnresolvedAgentishToken(
    messageBody,
    memberSet,
    rosterProfiles,
  );
  if (unresolvedAgentish) {
    throw new Error(
      memberSet.size === 0
        ? `@${unresolvedAgentish} is not a member of this conversation. Add the bot as a participant before @mentioning.`
        : `@${unresolvedAgentish} is not a member of this conversation. Only roster agents can be @mentioned.`,
    );
  }

  // Bare text: sole agent member may auto-route (Hub group + 1 human + 1 agent).
  // Sole token must resolve to an installed runtime bin for Host execution.
  const kind = (input.conversationKind ?? "group").toLowerCase();
  const humans = input.humanMemberCount ?? 1;
  if (kind === "group" && humans === 1 && members.length === 1) {
    const soleToken = members[0]!;
    const soleRuntime = resolveSoleRuntime(soleToken, rosterProfiles);
    if (!soleRuntime) {
      throw new Error(
        `Bot @${soleToken} is a conversation member but has no runnable runtime on this Host.`,
      );
    }
    if (!input.installedAgents.has(soleRuntime)) {
      throw new Error(
        `Bot @${soleRuntime} is a conversation member but not installed on this Host. Install the runtime or @mention only when available.`,
      );
    }
    if (!messageBody.trim()) {
      throw new Error("Cannot start an agent session with an empty prompt.");
    }
    targets.push({ agent: soleRuntime, prompt: messageBody });
    return { targets, multiRoutedCount: 0 };
  }

  // 0 agents → pure human IM; multi agents without @ → pure human (no fan-out).
  return { targets: [], multiRoutedCount: 0 };
}

/**
 * Build structured AppendMessage mentions from the message body.
 *
 * - Bot tokens reuse `parseAgentRouteTarget` against roster-scoped profiles
 *   (same parse surface as `resolveDispatchTargets` / `parseAllAgentRoutings`).
 * - `bot_id` = resolved profileId when present; otherwise the first roster
 *   profile whose runtime matches the bare agent token.
 * - Optional `start`/`length` are UTF-16 code-unit spans covering `@token`.
 * - Humans (`@minos_id`) are included when `options.mentionHumans` is provided.
 * - Appearance order; deduped by identity (`bot:` / `account:`).
 */
export function buildStructuredMentions(
  messageBody: string,
  mentionProfiles: readonly MentionProfile[],
  options?: BuildStructuredMentionsOptions,
): WireMentionTarget[] {
  const text = messageBody ?? "";
  if (!text) return [];

  const profiles = mentionProfiles ?? [];
  const humans = options?.mentionHumans ?? [];
  const humanByMinos = new Map(
    humans
      .map((h) => {
        const key = h.minosId.trim().toLowerCase();
        return key ? ([key, h] as const) : null;
      })
      .filter((x): x is readonly [string, MentionHuman] => x != null),
  );

  const out: WireMentionTarget[] = [];
  const seen = new Set<string>();

  // Same token surface as parseAllAgentRoutings / firstUnresolvedAgentishToken.
  const re = /(^|[\s([{"'`])@([^\s@]+)/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    const prefix = match[1] ?? "";
    const token = (match[2] ?? "").trim();
    if (!token) continue;
    const atStart = match.index + prefix.length;
    const spanLen = 1 + token.length; // include leading '@'

    // Prefer bot route parse (profile / runtime / p/<id>).
    const route = parseAgentRouteTarget(token, profiles);
    if (route) {
      const botId = resolveBotIdForRoute(route, profiles);
      if (botId) {
        const key = `bot:${botId}`;
        if (!seen.has(key)) {
          seen.add(key);
          out.push({
            kind: "bot",
            bot_id: botId,
            start: atStart,
            length: spanLen,
          });
        }
        continue;
      }
    }

    // Human @minos_id (only when roster humans provided).
    if (humans.length > 0) {
      const namePart = token.split("#")[0]?.trim() ?? token;
      const human = humanByMinos.get(namePart.toLowerCase());
      if (human?.accountId.trim()) {
        const accountId = human.accountId.trim();
        const key = `account:${accountId}`;
        if (!seen.has(key)) {
          seen.add(key);
          out.push({
            kind: "account",
            account_id: accountId,
            start: atStart,
            length: spanLen,
          });
        }
      }
    }
  }

  return out;
}

/** Resolve wire bot_id for a parsed route against roster profiles. */
function resolveBotIdForRoute(
  route: { agent: KnownAgent; profileId?: string },
  profiles: readonly MentionProfile[],
): string | null {
  if (route.profileId?.trim()) {
    const id = route.profileId.trim();
    // Prefer exact roster id; still accept parse-resolved id if caller passed it.
    const onRoster = profiles.find((p) => p.id === id);
    return onRoster?.id ?? id;
  }
  // Bare runtime → first roster profile with matching runtimeAgent.
  const runtime = route.agent.trim().toLowerCase();
  const byRuntime = profiles.find(
    (p) => p.runtimeAgent.trim().toLowerCase() === runtime,
  );
  if (byRuntime?.id.trim()) return byRuntime.id.trim();
  // No stable bot identity for bare runtime without roster profile.
  return null;
}

/** Map a sole roster token to a KnownAgent runtime for Host launch. */
function resolveSoleRuntime(
  token: string,
  profiles: readonly MentionProfile[],
): KnownAgent | null {
  const lower = token.trim().toLowerCase();
  if ((KNOWN_AGENTS as readonly string[]).includes(lower)) {
    return lower as KnownAgent;
  }
  const byId = profiles.find((p) => p.id.toLowerCase() === lower);
  if (byId) {
    const agent = byId.runtimeAgent.trim().toLowerCase();
    if ((KNOWN_AGENTS as readonly string[]).includes(agent)) {
      return agent as KnownAgent;
    }
  }
  const byName = profiles.find((p) => p.name.trim().toLowerCase() === lower);
  if (byName) {
    const agent = byName.runtimeAgent.trim().toLowerCase();
    if ((KNOWN_AGENTS as readonly string[]).includes(agent)) {
      return agent as KnownAgent;
    }
  }
  return null;
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
  const profileById = new Map(
    profiles.map((p) => [p.id.toLowerCase(), p]),
  );
  while ((match = re.exec(text)) !== null) {
    const raw = (match[2] ?? "").trim();
    if (!raw) continue;
    // Explicit p/<id> form
    const pidMatch = /^p[/:](.+)$/i.exec(raw);
    if (pidMatch) {
      const id = (pidMatch[1] ?? "").trim().toLowerCase();
      const profile = profileById.get(id);
      if (!profile) return raw;
      const ok =
        memberSet.has(profile.id.toLowerCase()) ||
        memberSet.has(profile.name.trim().toLowerCase()) ||
        memberSet.has(profile.runtimeAgent.trim().toLowerCase());
      if (!ok) return profile.name || raw;
      continue;
    }
    const namePart = raw.split("#")[0]?.trim() ?? raw;
    if (!namePart) continue;
    const lower = namePart.toLowerCase();
    if (memberSet.has(lower)) continue;
    const profile = profileByName.get(lower);
    if (profile) {
      const ok =
        memberSet.has(profile.id.toLowerCase()) ||
        memberSet.has(profile.name.trim().toLowerCase()) ||
        memberSet.has(profile.runtimeAgent.trim().toLowerCase());
      if (!ok) return namePart;
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
