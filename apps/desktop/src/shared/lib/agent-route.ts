/** Parse `@agent` / `@agent#short` / `@profile` routing (aligned with minos-tui agent_route.rs + desktop profiles). */

/**
 * Offline parse fallback for `@agent` tokens when CLI inventory is empty.
 * Production mention rows come from `buildAgentMentionOptions(clis, …)` using
 * daemon `list_clis`. Do **not** treat this as the runtime capability catalog —
 * see `features/agents/AGENTS.md` and domain `AgentName`.
 */
export const KNOWN_AGENTS = [
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
] as const;

export type KnownAgent = (typeof KNOWN_AGENTS)[number];

/**
 * Resolved @-route target.
 * - bare agent / agent#short: profileId unset (start uses newest profile default)
 * - profile mention: profileId + agent from profile.runtime_agent
 */
export type AgentRouteTarget = {
  agent: KnownAgent;
  sessionShortId?: string;
  /** Explicit host agent profile id when routed via @ProfileName or @p/<id>. */
  profileId?: string;
};

/** First up-to-8 chars of a session id (TUI `short_session_id` parity). */
export function shortSessionId(sessionId: string): string {
  let end = Math.min(8, sessionId.length);
  // Avoid splitting multi-byte code units at the cut (rare for hex ids).
  while (end > 0 && (sessionId.charCodeAt(end - 1) & 0xfc00) === 0xdc00) {
    end -= 1;
  }
  return sessionId.slice(0, end);
}

export function parseAgentName(value: string): KnownAgent | null {
  const normalized = value.toLowerCase();
  return (KNOWN_AGENTS as readonly string[]).includes(normalized)
    ? (normalized as KnownAgent)
    : null;
}

export type MentionProfile = {
  id: string;
  name: string;
  runtimeAgent: string;
};

/** Normalize a profile name for uniqueness / match (case-insensitive, trimmed). */
export function normalizeProfileName(name: string): string {
  return name.trim().toLowerCase();
}

/**
 * Whether a profile display name can be a single `@Name` route token.
 * Whitespace breaks token split; `#` is agent#session form; `@` nests mentions.
 * Daemon create/update rejects these; this is defense-in-depth for insert/parse.
 */
export function isProfileNameCleanToken(name: string): boolean {
  const n = name.trim();
  if (!n) return false;
  return !/[\s#@]/.test(n);
}

/**
 * Validate profile display name for create/update forms (daemon-aligned).
 * Returns an error message, or null when valid.
 */
export function validateProfileName(name: string): string | null {
  const n = name.trim();
  if (!n) return "Name is required";
  if (/[\s#@]/.test(n)) {
    return "Name cannot contain spaces, #, or @";
  }
  return null;
}

/**
 * Whether a profile display name is unique among profiles + runtime agent names.
 * Collisions force `@p/<id>` insert tokens so parse is unambiguous.
 */
export function isProfileNameUnique(
  name: string,
  profiles: readonly MentionProfile[],
  runtimeAgents: readonly string[] = KNOWN_AGENTS,
): boolean {
  const key = normalizeProfileName(name);
  if (!key) return false;
  if (runtimeAgents.some((a) => a.toLowerCase() === key)) return false;
  let hits = 0;
  for (const p of profiles) {
    if (normalizeProfileName(p.name) === key) hits += 1;
    if (hits > 1) return false;
  }
  return hits === 1;
}

/**
 * Insert token for a profile: `@Name ` when unique **and** a clean token,
 * else `@p/<id> ` (defense in depth for legacy / unsafe names).
 */
export function profileMentionInsert(
  profile: MentionProfile,
  profiles: readonly MentionProfile[],
  runtimeAgents: readonly string[] = KNOWN_AGENTS,
): string {
  if (
    isProfileNameCleanToken(profile.name) &&
    isProfileNameUnique(profile.name, profiles, runtimeAgents)
  ) {
    // Preserve user casing from profile.name for readability.
    return `@${profile.name.trim()} `;
  }
  return `@p/${profile.id} `;
}

/**
 * Parse a single route token (no leading `@`).
 *
 * Resolution order:
 * 1. `p/<id>` or `p:<id>` → profile by id (requires profiles list)
 * 2. `agent#short` → continue session
 * 3. known runtime agent name → bare agent
 * 4. unique profile name (case-insensitive) → profile
 */
export function parseAgentRouteTarget(
  value: string,
  profiles: readonly MentionProfile[] = [],
): AgentRouteTarget | null {
  const raw = value.trim();
  if (!raw) return null;

  // Explicit profile id form: p/<id> or p:<id>
  const pidMatch = /^p[/:](.+)$/i.exec(raw);
  if (pidMatch) {
    const id = pidMatch[1]?.trim();
    if (!id) return null;
    const profile = profiles.find((p) => p.id === id);
    if (!profile) {
      // Still accept token shape; agent unknown until profiles load — fail closed.
      return null;
    }
    const agent = parseAgentName(profile.runtimeAgent);
    if (!agent) return null;
    return { agent, profileId: profile.id };
  }

  const [agentPart, shortPart] = raw.split("#");
  if (shortPart !== undefined && shortPart.length === 0) return null;

  // Runtime agents win over same-named profiles (colliding profiles use @p/id).
  const agent = parseAgentName(agentPart ?? "");
  if (agent) {
    return {
      agent,
      sessionShortId: shortPart || undefined,
    };
  }

  // Profile by unique name (case-insensitive). Ignore when #session form.
  if (shortPart !== undefined) return null;
  const key = normalizeProfileName(agentPart ?? "");
  if (!key) return null;
  const nameMatches = profiles.filter(
    (p) => normalizeProfileName(p.name) === key,
  );
  if (nameMatches.length !== 1) return null;
  const profile = nameMatches[0]!;
  const profileAgent = parseAgentName(profile.runtimeAgent);
  if (!profileAgent) return null;
  return { agent: profileAgent, profileId: profile.id };
}

/**
 * `@codex hello` → { target: { agent: "codex" }, prompt: "hello", messageBody: "@codex hello" }
 * `@ResearchGrok hello` → profile target when name unique
 * `@p/profile-uuid hello` → profile by id
 * plain text → null (caller may fall back to default agent)
 *
 * Routing semantics:
 * - `@agent prompt` → start/reuse session for that agent (newest profile default on create)
 * - `@agent#short prompt` → continue an existing session
 * - `@ProfileName` / `@p/<id>` → **new** session with explicit profile_id
 */
export function parseAgentRouting(
  text: string,
  profiles: readonly MentionProfile[] = [],
): { target: AgentRouteTarget; prompt: string; messageBody: string } | null {
  return parseAllAgentRoutings(text, profiles)[0] ?? null;
}

export type AgentRouting = {
  target: AgentRouteTarget;
  prompt: string;
  messageBody: string;
};

/**
 * Multi-@ fan-out: every unique agent mentioned in the body (appearance order).
 * Shared prompt is the body with leading agent @tokens stripped when the
 * message starts with @; otherwise the full text is the prompt for each agent.
 */
export function parseAllAgentRoutings(
  text: string,
  profiles: readonly MentionProfile[] = [],
): AgentRouting[] {
  const messageBody = text.trimEnd();
  const trimmed = text.trimStart();
  if (!trimmed) return [];

  const routes: AgentRouting[] = [];
  const seen = new Set<string>();

  // Walk @tokens anywhere in the body (not only leading).
  const re = /(^|[\s([{"'`])@([^\s@]+)/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(trimmed)) !== null) {
    const token = match[2] ?? "";
    if (!token) continue;
    const target = parseAgentRouteTarget(token, profiles);
    if (!target) continue;
    const key = target.profileId
      ? `p:${target.profileId}`
      : target.sessionShortId
        ? `${target.agent}#${target.sessionShortId}`
        : target.agent;
    if (seen.has(key)) continue;
    // Dedup by agent runtime for bare @agent (one session per agent per turn).
    const agentKey = target.agent;
    if (!target.sessionShortId && !target.profileId && seen.has(`a:${agentKey}`)) {
      continue;
    }
    seen.add(key);
    if (!target.sessionShortId && !target.profileId) {
      seen.add(`a:${agentKey}`);
    }
    routes.push({ target, prompt: "", messageBody });
  }

  if (routes.length === 0) return [];

  // Shared prompt: strip leading consecutive agent @tokens.
  let prompt = trimmed;
  while (prompt.startsWith("@")) {
    const rest = prompt.slice(1);
    const splitAt = [...rest].findIndex((ch) => /\s/.test(ch));
    const token = splitAt === -1 ? rest : rest.slice(0, splitAt);
    if (!parseAgentRouteTarget(token, profiles)) break;
    prompt = splitAt === -1 ? "" : rest.slice(splitAt).trimStart();
  }
  // Also strip mid-body agent mentions when message is pure multi-@ + instruction
  // is already handled by leading strip; keep full body if no leading @.
  const sharedPrompt = prompt.trim() || messageBody;

  return routes.map((r) => ({ ...r, prompt: sharedPrompt }));
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

export type BuildAgentMentionOptionsArgs = {
  query: string;
  clis: readonly MentionCli[];
  sessions: readonly MentionSession[];
  profiles?: readonly MentionProfile[];
  /**
   * Conversation roster (membership SSOT). When provided, only these runtime
   * agents (and profiles/sessions for them) appear. Empty array ⇒ no options.
   * Omit only for offline/unit contexts that intentionally list all CLIs.
   */
  memberAgents?: readonly string[];
  limit?: number;
};

/**
 * TUI-parity @-picker rows (membership-gated when `memberAgents` is set):
 * 1. Member runtimes as bare `@agent` → start (newest profile default)
 * 2. Host profiles for member runtimes as `@ProfileName` / `@p/id`
 * 3. Existing open member sessions as `@agent#short` → continue
 */
export function buildAgentMentionOptions(
  queryOrArgs: string | BuildAgentMentionOptionsArgs,
  clisArg?: readonly MentionCli[],
  sessionsArg?: readonly MentionSession[],
  profilesArg: readonly MentionProfile[] = [],
  limitArg = 16,
): MentionOption[] {
  // Support both positional (legacy tests) and object form.
  const args: BuildAgentMentionOptionsArgs =
    typeof queryOrArgs === "string"
      ? {
          query: queryOrArgs,
          clis: clisArg ?? [],
          sessions: sessionsArg ?? [],
          profiles: profilesArg,
          limit: limitArg,
        }
      : queryOrArgs;

  const query = args.query;
  const clis = args.clis;
  const sessions = args.sessions;
  const profiles = args.profiles ?? [];
  const limit = args.limit ?? 16;
  const memberFilter =
    args.memberAgents === undefined
      ? null
      : new Set(
          args.memberAgents
            .map((a) => a.trim().toLowerCase())
            .filter((a) => a.length > 0),
        );

  const isMember = (agent: string) =>
    memberFilter === null || memberFilter.has(agent.toLowerCase());

  // Explicit empty roster: nothing is @-able.
  if (memberFilter !== null && memberFilter.size === 0) {
    return [];
  }

  const q = query.toLowerCase();
  const matches = (s: string) => !q || s.toLowerCase().includes(q);

  const memberClis = clis.filter((c) => isMember(c.agent));
  const fromCli = memberClis
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
      : memberFilter === null
        ? KNOWN_AGENTS.filter((a) => matches(a) || matches(`@${a}`)).map(
            (a) => ({
              id: `new:${a}`,
              label: `@${a}`,
              hint: "new session",
              insert: `@${a} `,
              disabled: false,
            }),
          )
        : // Roster set but CLI inventory empty: still offer bare member tokens.
          [...memberFilter]
            .filter((a) => matches(a) || matches(`@${a}`))
            .map((a) => ({
              id: `new:${a}`,
              label: `@${a}`,
              hint: "new session",
              insert: `@${a} `,
              disabled: false,
            }));

  const runtimeNames = [
    ...new Set([
      ...KNOWN_AGENTS,
      ...clis.map((c) => c.agent),
      ...(memberFilter ? [...memberFilter] : []),
    ]),
  ];

  const fromProfiles = profiles
    .filter((p) => isMember(p.runtimeAgent))
    .filter(
      (p) =>
        matches(p.name) ||
        matches(p.runtimeAgent) ||
        matches(`@${p.name}`) ||
        matches(`p/${p.id}`),
    )
    .map((p) => {
      const useName =
        isProfileNameCleanToken(p.name) &&
        isProfileNameUnique(p.name, profiles, runtimeNames);
      const label = useName
        ? `@${p.name.trim()}`
        : `@p/${p.id.slice(0, 12)}…`;
      return {
        id: `profile:${p.id}`,
        label,
        hint: `profile · ${p.runtimeAgent}`,
        insert: profileMentionInsert(p, profiles, runtimeNames),
        disabled: false,
      };
    });

  const fromSessions = sessions
    .filter((s) => isMember(s.agent))
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

  return [...fromKnown, ...fromProfiles, ...fromSessions].slice(0, limit);
}
