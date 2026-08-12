/**
 * Per-kind outbox post implementations (Account WS / Hub REST / daemon).
 */

import { getCloudAuth } from "@/shared/lib/cloud-auth";
import { getAccountScopeGeneration } from "@/shared/lib/account-scope-generation";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";
import { sessionIdFromAgentResultId } from "@/shared/lib/cloud-timeline";
import {
  respondCloudApproval,
  sendAgentConversationMessage,
  toggleCloudReaction,
} from "@/shared/lib/minos-cloud";
import type { ImOutboxEntry } from "@/shared/lib/im-outbox";
import { daemonApi } from "@/shared/lib/daemon";
import { appendMessageOnCloud } from "@/store/im/im-cloud-bridge";
import {
  attachAgentsToConversationCloud,
  resolveCloudAgentId,
  RUNTIMES_FROM_ID,
  syncConversationToCloud,
} from "@/store/im/im-cloud-agents";

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

export type CloudUserMessageAck = {
  messageId: string;
  messageSeq: number;
  conversationId: string;
};

async function postUserMessageFromOutbox(
  entry: ImOutboxEntry,
  scope: { accountId: string; scopeGeneration: number },
): Promise<CloudUserMessageAck> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("Not signed in — message not synced to cloud");
  }
  if (
    auth.accountId !== scope.accountId ||
    entry.accountId.trim() !== scope.accountId ||
    getAccountScopeGeneration() !== scope.scopeGeneration
  ) {
    throw new Error("Outbox send aborted: account scope changed");
  }

  // Never upsert Hub with the "Conversation" placeholder — that clobbers real
  // titles on every host_projection message. Only push a real title; agents-only
  // attach uses attachAgentsToConversationCloud when title is missing.
  const realTitle = entry.title?.trim() ?? "";
  const isPlaceholder =
    !realTitle || realTitle.toLowerCase() === "conversation";
  if (!isPlaceholder) {
    await syncConversationToCloud({
      conversationId: entry.conversationId,
      title: realTitle,
      agentRuntimes: entry.agentRuntimes,
    });
  } else if ((entry.agentRuntimes?.length ?? 0) > 0) {
    await attachAgentsToConversationCloud({
      conversationId: entry.conversationId,
      agentRuntimes: entry.agentRuntimes ?? [],
    });
  }

  if (
    getAccountScopeGeneration() !== scope.scopeGeneration ||
    cloudAuth()?.accountId !== scope.accountId
  ) {
    throw new Error("Outbox send aborted: account scope changed");
  }

  // Linked Hub-first sends use client_live so Hub can @-dispatch.
  // Entries without explicit source default to client_live.
  const messageSource = entry.messageSource ?? "client_live";

  // Account WS AppendMessage only (no REST collaboration write path).
  // Wait for ChatSendAck before treating outbox as success.
  // Socket/timeout → retry via outbox; definitive nack fails the entry.
  if (messageSource !== "client_live") {
    // host_projection / system user rows are not collaboration writes here.
    // Agent final text uses postAgentResultFromOutbox.
    throw new Error(
      `Unsupported user-message source for Hub uplink: ${messageSource}`,
    );
  }

  const wsResult = await appendMessageOnCloud({
    clientOperationId: entry.clientMessageId,
    conversationId: entry.conversationId,
    text: entry.text,
    replyToMessageId: entry.replyToMessageId,
    expectedAccountId: scope.accountId,
    mentions: entry.mentions,
  });
  if (wsResult.ok) {
    if (
      getAccountScopeGeneration() !== scope.scopeGeneration ||
      cloudAuth()?.accountId !== scope.accountId
    ) {
      throw new Error("Outbox send aborted: account scope changed");
    }
    return {
      messageId: wsResult.messageId,
      messageSeq: wsResult.messageSeq,
      conversationId: wsResult.conversationId,
    };
  }
  if (wsResult.reason === "nack") {
    throw new Error(
      `ChatSendNack: ${wsResult.code ?? "nack"}${
        wsResult.message ? ` — ${wsResult.message}` : ""
      }`,
    );
  }
  throw new Error(
    wsResult.reason === "timeout"
      ? "AppendMessage timed out waiting for ChatSendAck"
      : "AppendMessage unavailable (Account WS not live)",
  );
}

async function postAgentResultFromOutbox(entry: ImOutboxEntry): Promise<void> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("Not signed in — agent result not synced to cloud");
  }

  let agentId = entry.agentId?.trim() || null;
  if (!agentId) {
    const runtime =
      entry.agentRuntimes?.find((r) => !!r?.trim()) ??
      RUNTIMES_FROM_ID.find((r) =>
        entry.clientMessageId.toLowerCase().includes(r),
      ) ??
      null;
    if (runtime) {
      agentId = await resolveCloudAgentId(runtime);
    }
  }
  if (!agentId) {
    throw new Error("agent_result uplink: no cloud agent id");
  }

  const sessionId =
    entry.agentSessionId?.trim() ||
    sessionIdFromAgentResultId(entry.clientMessageId) ||
    undefined;

  await sendAgentConversationMessage(
    auth.deviceId,
    auth.accessToken,
    entry.conversationId,
    {
      agentId,
      text: entry.text,
      clientMessageId: entry.clientMessageId,
      agentSessionId: sessionId,
      replyToMessageId: entry.replyToMessageId ?? undefined,
      messageSource: "host_projection",
    },
  );
}

export type CloudReactionToggleResult = {
  messageId: string;
  conversationId: string;
  action: string;
  reactions: Array<{
    emoji: string;
    count: number;
    reactedByMe: boolean;
    actors: Array<{
      actorId: string;
      actorKind: string;
      displayName: string;
    }>;
  }>;
};

async function postReactionToggleFromOutbox(
  entry: ImOutboxEntry,
): Promise<CloudReactionToggleResult> {
  const auth = cloudAuth();
  if (!auth) {
    throw new Error("not authenticated");
  }
  let payload: { messageId?: string; emoji?: string };
  try {
    payload = JSON.parse(entry.text) as { messageId?: string; emoji?: string };
  } catch {
    throw new Error("invalid_payload: reaction_toggle text is not JSON");
  }
  const messageId = payload.messageId?.trim() ?? "";
  const emoji = payload.emoji?.trim() ?? "";
  if (!messageId || !emoji) {
    throw new Error("invalid_payload: reaction_toggle requires messageId+emoji");
  }
  // clientMessageId == wire client_op_id; retries reuse the same id.
  return toggleCloudReaction(
    auth.deviceId,
    auth.accessToken,
    entry.conversationId,
    messageId,
    emoji,
    entry.clientMessageId,
  );
}

/**
 * Treat daemon/Hub "already resolved" as success so outbox retries after a
 * successful-but-unacked first attempt do not burn the entry.
 */
function isApprovalAlreadyResolvedError(error: unknown): boolean {
  const code =
    error && typeof error === "object" && "code" in error
      ? String((error as { code: unknown }).code ?? "").toLowerCase()
      : "";
  const msg = (error instanceof Error ? error.message : String(error)).toLowerCase();
  // Only treat explicit already-resolved / approval_not_found as success.
  // Broad "not found" would mask unrelated 404s and burn retries incorrectly.
  return (
    code.includes("already_resolved") ||
    code.includes("approval_already") ||
    code.includes("approval_not_found") ||
    msg.includes("already_resolved") ||
    msg.includes("already resolved") ||
    msg.includes("approval_already") ||
    msg.includes("approval_not_found") ||
    msg.includes("approval not found")
  );
}

/**
 * Branch by auth mode (no dual SSOT):
 * - Hub IM mode: POST /v1/approvals/respond + top-level client_request_id
 * - Local-only: daemon resolveApproval; decision JSON never carries client_request_id
 */
async function postApprovalResolveFromOutbox(
  entry: ImOutboxEntry,
): Promise<void> {
  let payload: {
    requestId?: string;
    sessionId?: string;
    decision?: string | Record<string, unknown>;
    /** Explicit route stamped at enqueue (`hub` | `daemon`). */
    route?: string;
  };
  try {
    payload = JSON.parse(entry.text) as typeof payload;
  } catch {
    throw new Error("invalid_payload: approval_resolve text is not JSON");
  }
  const requestId = payload.requestId?.trim() ?? "";
  const sessionId = payload.sessionId?.trim() ?? "";
  if (!requestId || !sessionId || payload.decision == null) {
    throw new Error(
      "invalid_payload: approval_resolve requires requestId+sessionId+decision",
    );
  }
  // Clean agent decision only — never nest client_request_id in decision JSON.
  const decision =
    typeof payload.decision === "string"
      ? { decision: payload.decision }
      : { ...payload.decision };
  delete (decision as Record<string, unknown>).client_request_id;

  const authSnap = getCloudAuth();
  const cloudRoute =
    payload.route === "hub" ||
    (payload.route !== "daemon" &&
      isCloudImMode({
        authPhase: authSnap?.authPhase,
        accessToken: authSnap?.accessToken,
      }));

  try {
    if (cloudRoute) {
      const auth = cloudAuth();
      if (!auth) {
        throw new Error("not authenticated");
      }
      // Top-level client_request_id = outbox logical op id (Intent Outbox).
      await respondCloudApproval(auth.deviceId, auth.accessToken, {
        requestId,
        decision,
        clientRequestId: entry.clientMessageId,
      });
    } else {
      await daemonApi.resolveApproval(requestId, sessionId, decision);
    }
  } catch (error) {
    if (isApprovalAlreadyResolvedError(error)) {
      return;
    }
    throw error;
  }
}

export type OutboxScope = { accountId: string; scopeGeneration: number };

export async function postOutboxEntry(
  entry: ImOutboxEntry,
  scope: OutboxScope,
): Promise<CloudReactionToggleResult | CloudUserMessageAck | void> {
  if (
    entry.accountId.trim() !== scope.accountId ||
    getAccountScopeGeneration() !== scope.scopeGeneration ||
    cloudAuth()?.accountId !== scope.accountId
  ) {
    throw new Error("Outbox send aborted: account scope changed");
  }
  switch (entry.kind) {
    case "user_message":
      return postUserMessageFromOutbox(entry, scope);
    case "agent_result":
      await postAgentResultFromOutbox(entry);
      return;
    case "reaction_toggle":
      return postReactionToggleFromOutbox(entry);
    case "approval_resolve":
      await postApprovalResolveFromOutbox(entry);
      return;
    default: {
      const _exhaustive: never = entry.kind;
      throw new Error(`unknown outbox kind: ${_exhaustive}`);
    }
  }
}

/**
 * Single path: enqueue reaction_toggle then drain via lane worker.
 * Returns Hub aggregate for generation-gated apply (from worker side map).
 * Logical op id = clientOpId (= wire client_op_id); row id = outbox:${clientOpId}.
 */
