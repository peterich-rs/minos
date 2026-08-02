/**
 * Hub → Desktop IM inbound (Phase 3–4 / 6.0).
 *
 * Map Hub messages into the workspace timeline projection only.
 * Never append cloud IM into Host daemon SQLite (collaboration SSOT = Hub).
 *
 * Cold open / realtime both use `hubChatMessageToTimeline` + store merge.
 * Gap fill uses `before_ts_ms` (messages/query).
 */

import {
  listHubConversationMessages,
  listCloudAgents,
  type HubChatMessage,
  type HubMessagePage,
} from "@/shared/lib/minos-cloud";
import { hubChatMessageToTimeline } from "@/shared/lib/hub-timeline";
import { useAccountStore } from "@/store/account-store";
import { markMessageProjected } from "@/shared/lib/im-cloud-sync";
import type { TimelineMessage } from "@/shared/lib/mock-data";
import { MESSAGE_PAGE_SIZE } from "@/shared/lib/message-history";

/** cloud agent_id → runtime bin name (codex/claude/…) */
const agentIdToRuntime = new Map<string, string>();
let agentsLoadedAt = 0;
const AGENTS_TTL_MS = 60_000;

function cloudAuth(): { deviceId: string; accessToken: string } | null {
  const { deviceId, session } = useAccountStore.getState();
  if (!session?.accessToken?.trim()) return null;
  return { deviceId, accessToken: session.accessToken };
}

export async function ensureAgentRuntimeMap(): Promise<Map<string, string>> {
  const auth = cloudAuth();
  if (!auth) return agentIdToRuntime;
  if (Date.now() - agentsLoadedAt < AGENTS_TTL_MS && agentIdToRuntime.size > 0) {
    return agentIdToRuntime;
  }
  try {
    const agents = await listCloudAgents(auth.deviceId, auth.accessToken);
    agentIdToRuntime.clear();
    for (const a of agents) {
      agentIdToRuntime.set(a.agentId, a.runtimeAgent.toLowerCase());
    }
    agentsLoadedAt = Date.now();
  } catch (error) {
    console.warn("[im-cloud-inbound] list agents failed", error);
  }
  return agentIdToRuntime;
}

/** Map one hub message to a timeline row (no daemon write). */
export async function mapHubChatMessageToTimeline(
  message: HubChatMessage,
): Promise<TimelineMessage | null> {
  await ensureAgentRuntimeMap();
  return hubChatMessageToTimeline(message, {
    agentRuntimeMap: agentIdToRuntime,
  });
}

function mapPageToTimeline(page: HubMessagePage): {
  messages: TimelineMessage[];
  nextBeforeTsMs: number | null;
  rawCount: number;
} {
  const ordered = page.messages
    .slice()
    .sort((a, b) => a.createdAtMs - b.createdAtMs);
  const out: TimelineMessage[] = [];
  for (const m of ordered) {
    markMessageProjected(m.messageId);
    const row = hubChatMessageToTimeline(m, {
      agentRuntimeMap: agentIdToRuntime,
    });
    if (row) out.push(row);
  }
  return {
    messages: out,
    nextBeforeTsMs: page.nextBeforeTsMs,
    rawCount: page.messages.length,
  };
}

/**
 * Cold pull hub messages for a conversation → TimelineMessage[] (Hub SSOT).
 * Does **not** write into Host daemon chat_messages.
 */
export async function pullHubConversationMessages(
  conversationId: string,
  opts?: { beforeTsMs?: number; limit?: number },
): Promise<TimelineMessage[]> {
  const page = await pullHubConversationMessagePage(conversationId, opts);
  return page.messages;
}

/** Cold pull with gap cursor (Phase 4.4). */
export async function pullHubConversationMessagePage(
  conversationId: string,
  opts?: { beforeTsMs?: number; limit?: number },
): Promise<{
  messages: TimelineMessage[];
  nextBeforeTsMs: number | null;
  rawCount: number;
}> {
  const auth = cloudAuth();
  if (!auth || !conversationId.trim()) {
    return { messages: [], nextBeforeTsMs: null, rawCount: 0 };
  }
  try {
    const page = await listHubConversationMessages(
      auth.deviceId,
      auth.accessToken,
      conversationId,
      {
        beforeTsMs: opts?.beforeTsMs,
        limit: opts?.limit ?? MESSAGE_PAGE_SIZE,
      },
    );
    await ensureAgentRuntimeMap();
    return mapPageToTimeline(page);
  } catch (error) {
    console.warn("[im-cloud-inbound] pull hub conversation failed", error);
    return { messages: [], nextBeforeTsMs: null, rawCount: 0 };
  }
}
