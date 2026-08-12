/**
 * Pure helpers for the create-conversation dialog.
 * Keeps form defaults / validation out of the React component.
 */

import type { ConversationPriority } from "@/shared/domain/collaboration";

/** How the conversation binds to git on create. */
export type ConversationGitMode = "worktree" | "inherit";

/** One roster member at create time (runtime + optional peer-facing brief). */
export type ConversationAgentSpec = {
  agent: string;
  /** Peer-facing role description (≤500 chars). */
  brief?: string | null;
};

export type CreateConversationFormInput = {
  title: string;
  /** Unset when null/undefined. */
  priority?: ConversationPriority | null;
  /** Roster members with optional briefs. */
  agents?: ConversationAgentSpec[];
  /**
   * Git isolation mode.
   * - `worktree` (default): dedicated branch + linked worktree when project is a git repo
   * - `inherit`: use the project workspace checkout as-is
   */
  gitMode?: ConversationGitMode;
};

export type CreateConversationAgentOption = {
  id: string;
  displayName: string;
  installed: boolean;
};

export const CREATE_CONVERSATION_PRIORITIES: Array<{
  value: ConversationPriority | null;
  label: string;
}> = [
  { value: null, label: "None" },
  { value: "high", label: "High" },
  { value: "medium", label: "Medium" },
  { value: "low", label: "Low" },
];

export const CREATE_CONVERSATION_GIT_MODES: Array<{
  value: ConversationGitMode;
  label: string;
  description: string;
}> = [
  {
    value: "worktree",
    label: "Isolated worktree",
    description:
      "Create a linked worktree and branch so agents do not touch the default branch.",
  },
  {
    value: "inherit",
    label: "Project workspace",
    description: "Use the project checkout as-is (shared working tree).",
  },
];

/** Max length for peer-facing roster briefs (matches daemon store cap). */
export const MAX_AGENT_BRIEF_CHARS = 500;

/** Default form values when the dialog opens. Agents start empty (opt-in). */
export function defaultCreateConversationForm(): {
  title: string;
  priority: ConversationPriority | null;
  selectedAgents: string[];
  agentBriefs: Record<string, string>;
  gitMode: ConversationGitMode;
} {
  return {
    title: "",
    priority: null,
    selectedAgents: [],
    agentBriefs: {},
    gitMode: "worktree",
  };
}

export function normalizeCreateConversationTitle(title: string): string {
  return title.trim().replace(/\s+/g, " ");
}

export function normalizeAgentBrief(brief: string | null | undefined): string {
  const trimmed = (brief ?? "").trim().replace(/\s+/g, " ");
  if (!trimmed) return "";
  if (trimmed.length <= MAX_AGENT_BRIEF_CHARS) return trimmed;
  return trimmed.slice(0, MAX_AGENT_BRIEF_CHARS);
}

/** Minimal profile fields used to default roster briefs. */
export type ProfileBriefSource = {
  runtime_agent: string;
  description?: string | null;
  /** Prefer higher when multiple profiles share a runtime. */
  updated_at_ms?: number | null;
};

/**
 * Map runtime agent id → newest non-empty profile description.
 * Profile description is the Host-level peer-facing role brief.
 */
export function defaultBriefsFromProfiles(
  profiles: readonly ProfileBriefSource[],
): Record<string, string> {
  const best = new Map<string, { brief: string; updated: number }>();
  for (const p of profiles) {
    const runtime = (p.runtime_agent ?? "").trim().toLowerCase();
    const brief = normalizeAgentBrief(p.description);
    if (!runtime || !brief) continue;
    const updated =
      typeof p.updated_at_ms === "number" && Number.isFinite(p.updated_at_ms)
        ? p.updated_at_ms
        : 0;
    const prev = best.get(runtime);
    if (!prev || updated >= prev.updated) {
      best.set(runtime, { brief, updated });
    }
  }
  const out: Record<string, string> = {};
  for (const [runtime, v] of best) {
    out[runtime] = v.brief;
  }
  return out;
}

export function canSubmitCreateConversation(
  title: string,
  selectedAgents: readonly string[] = [],
): boolean {
  // Require at least one roster agent so @mention is possible after create.
  return (
    normalizeCreateConversationTitle(title).length > 0 &&
    selectedAgents.some((a) => a.trim().length > 0)
  );
}

/**
 * Toggle an agent id in the multi-select set.
 * Unknown / empty ids are ignored. Order follows existing selection + append.
 */
export function toggleSelectedAgent(
  selected: readonly string[],
  agentId: string,
): string[] {
  const id = agentId.trim();
  if (!id) return [...selected];
  if (selected.includes(id)) {
    return selected.filter((a) => a !== id);
  }
  return [...selected, id];
}

/**
 * Keep only known agent ids (installed preferred but uninstalled may remain if
 * already selected while inventory refreshes). Dedupes while preserving order.
 */
export function sanitizeSelectedAgents(
  selected: readonly string[],
  options: readonly CreateConversationAgentOption[],
): string[] {
  const known = new Set(options.map((o) => o.id));
  const out: string[] = [];
  for (const id of selected) {
    const trimmed = id.trim();
    if (!trimmed || !known.has(trimmed) || out.includes(trimmed)) continue;
    out.push(trimmed);
  }
  return out;
}

export function normalizeGitMode(
  value: string | null | undefined,
): ConversationGitMode {
  return value === "inherit" ? "inherit" : "worktree";
}

export function buildCreateConversationInput(form: {
  title: string;
  priority: ConversationPriority | null;
  selectedAgents: readonly string[];
  agentBriefs?: Readonly<Record<string, string>>;
  gitMode?: ConversationGitMode | null;
}): CreateConversationFormInput | null {
  const title = normalizeCreateConversationTitle(form.title);
  if (!title) return null;
  const briefs = form.agentBriefs ?? {};
  const agents: ConversationAgentSpec[] = [];
  const seen = new Set<string>();
  for (const raw of form.selectedAgents) {
    const agent = raw.trim().toLowerCase();
    if (!agent || seen.has(agent)) continue;
    seen.add(agent);
    const brief = normalizeAgentBrief(briefs[raw] ?? briefs[agent]);
    agents.push(brief ? { agent, brief } : { agent });
  }
  return {
    title,
    priority: form.priority ?? null,
    agents,
    gitMode: normalizeGitMode(form.gitMode),
  };
}
