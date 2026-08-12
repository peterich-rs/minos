/**
 * Resolve host runtime → Hub agent_id and conversation shell upsert.
 */

import { getCloudAuth } from "@/shared/lib/cloud-auth";
import {
  addAgentToConversation,
  ensureHostRuntimeAgent,
  upsertConversation,
} from "@/shared/lib/minos-cloud";
import {
  displayNameForRuntime,
  normalizeHostRuntime,
} from "@/shared/lib/im-cloud-sync-helpers";

const runtimeAgentIdCache = new Map<string, string>();

export function clearRuntimeAgentIdCache(): void {
  runtimeAgentIdCache.clear();
}

function cloudAuth(): {
  deviceId: string;
  accessToken: string;
  accountId: string;
} | null {
  const auth = getCloudAuth();
  if (!auth) return null;
  const accessToken = auth.accessToken.trim();
  const accountId = auth.accountId.trim();
  if (!accessToken || !accountId) return null;
  return {
    deviceId: auth.deviceId,
    accessToken,
    accountId,
  };
}

function normalizeRuntime(runtime: string | null | undefined): string | null {
  return normalizeHostRuntime(runtime);
}

export async function resolveCloudAgentId(
  runtimeAgent: string,
): Promise<string | null> {
  const runtime = normalizeRuntime(runtimeAgent);
  if (!runtime) return null;
  const cached = runtimeAgentIdCache.get(runtime);
  if (cached) return cached;

  const auth = cloudAuth();
  if (!auth) return null;

  try {
    const agent = await ensureHostRuntimeAgent(auth.deviceId, auth.accessToken, {
      runtimeAgent: runtime,
      name: displayNameForRuntime(runtime),
    });
    runtimeAgentIdCache.set(runtime, agent.agentId);
    return agent.agentId;
  } catch (error) {
    console.warn("[im-cloud-sync] ensure host runtime agent failed", runtime, error);
    return null;
  }
}

export async function resolveCloudAgentIds(
  runtimes: Array<string | null | undefined>,
): Promise<string[]> {
  const unique = new Set<string>();
  for (const r of runtimes) {
    const n = normalizeRuntime(r);
    if (n) unique.add(n);
  }
  const ids: string[] = [];
  for (const runtime of unique) {
    const id = await resolveCloudAgentId(runtime);
    if (id) ids.push(id);
  }
  return ids;
}

/** Register / refresh a Desktop work conversation on the hub (shell + roster). */
export async function syncConversationToCloud(input: {
  conversationId: string;
  title: string;
  memberAccountIds?: string[];
  /** Local runtime names (codex/claude/…); resolved to cloud agent ids. */
  agentRuntimes?: Array<string | null | undefined>;
  /** Pre-resolved cloud agent ids (optional; merged with agentRuntimes). */
  agentIds?: string[];
}): Promise<void> {
  const auth = cloudAuth();
  if (!auth) return;
  const title = input.title.trim();
  if (!title || !input.conversationId.trim()) return;

  const fromRuntimes = await resolveCloudAgentIds(input.agentRuntimes ?? []);
  const agentIds = [
    ...new Set([...(input.agentIds ?? []), ...fromRuntimes].filter(Boolean)),
  ];

  try {
    await upsertConversation(auth.deviceId, auth.accessToken, {
      conversationId: input.conversationId,
      title,
      memberAccountIds: input.memberAccountIds ?? [],
      agentIds,
    });
  } catch (error) {
    console.warn("[im-cloud-sync] upsert conversation failed", error);
  }
}

/**
 * Attach host-runtime agents to an existing hub conversation without touching
 * the title (used when starting a session mid-conversation).
 */
export async function attachAgentsToConversationCloud(input: {
  conversationId: string;
  agentRuntimes: Array<string | null | undefined>;
}): Promise<void> {
  const auth = cloudAuth();
  if (!auth) return;
  if (!input.conversationId.trim()) return;
  const agentIds = await resolveCloudAgentIds(input.agentRuntimes);
  for (const agentId of agentIds) {
    try {
      await addAgentToConversation(
        auth.deviceId,
        auth.accessToken,
        input.conversationId,
        agentId,
      );
    } catch (error) {
      console.warn(
        "[im-cloud-sync] add agent to conversation failed",
        agentId,
        error,
      );
    }
  }
}

export const RUNTIMES_FROM_ID = [
  "codex",
  "claude",
  "gemini",
  "opencode",
  "grok",
] as const;

