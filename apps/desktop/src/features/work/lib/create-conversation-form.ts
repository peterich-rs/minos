/**
 * Pure helpers for the create-conversation dialog.
 * Keeps form defaults / validation out of the React component.
 */

import type { ConversationPriority } from "@/shared/lib/mock-data";

export type CreateConversationFormInput = {
  title: string;
  /** Unset when null/undefined. */
  priority?: ConversationPriority | null;
  /** Runtime agent ids to start after the conversation is created. */
  agents?: string[];
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

/** Default form values when the dialog opens. Agents start empty (opt-in). */
export function defaultCreateConversationForm(): {
  title: string;
  priority: ConversationPriority | null;
  selectedAgents: string[];
} {
  return {
    title: "",
    priority: null,
    selectedAgents: [],
  };
}

export function normalizeCreateConversationTitle(title: string): string {
  return title.trim().replace(/\s+/g, " ");
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

export function buildCreateConversationInput(form: {
  title: string;
  priority: ConversationPriority | null;
  selectedAgents: readonly string[];
}): CreateConversationFormInput | null {
  const title = normalizeCreateConversationTitle(form.title);
  if (!title) return null;
  const agents = form.selectedAgents
    .map((a) => a.trim())
    .filter((a, i, arr) => a.length > 0 && arr.indexOf(a) === i);
  return {
    title,
    priority: form.priority ?? null,
    agents,
  };
}
