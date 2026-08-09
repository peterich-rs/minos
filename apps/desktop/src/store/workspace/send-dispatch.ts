/**
 * Shared Desktop user-turn dispatch: target resolution + multi-@ fan-out.
 *
 * First send and retry must share the same intent semantics:
 *   1. resolve targets (may throw — no durable append yet)
 *   2. append user message
 *   3. mark bubble sent
 *   4. fan-out agent turns (failures never re-fail a durable bubble)
 */

import type { WorkspaceGet, WorkspaceSet } from "./types";
import { commitSessionEntity, findSessionRow } from "./projection";
import { patchSessionEntity } from "@/shared/lib/session-entity";
import { daemonApi } from "@/shared/lib/daemon";
import {
  parseAllAgentRoutings,
  type KnownAgent,
  type MentionProfile,
} from "@/shared/lib/agent-route";
import { ensureSessionsForRouting, startNewAgentSession } from "./shared";

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
 * Resolve multi-@ or default-member targets. Throws before any append.
 */
export function resolveDispatchTargets(
  input: ResolveDispatchTargetsInput,
): { targets: DispatchTarget[]; multiRoutedCount: number } {
  const messageBody = input.messageBody;
  const multiRouted = parseAllAgentRoutings(messageBody, input.mentionProfiles);
  const members = new Set(
    (input.participatingAgents ?? []).map((a) => a.toLowerCase()),
  );

  const targets: DispatchTarget[] = [];
  if (multiRouted.length > 0) {
    for (const routed of multiRouted) {
      const agent = routed.target.agent;
      if (!members.has(agent)) {
        throw new Error(
          members.size === 0
            ? "No agents in this conversation. Select agents when creating it before @mentioning."
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
  } else {
    const firstMember = (input.participatingAgents ?? []).find((name) =>
      input.installedAgents.has(name.toLowerCase()),
    );
    const agent = (firstMember as KnownAgent | undefined) ?? null;
    if (!agent) {
      throw new Error(
        members.size === 0
          ? "No agents in this conversation. Select agents when creating it."
          : "No installed agents among conversation members. Install a member runtime or recreate with different agents.",
      );
    }
    if (!messageBody.trim()) {
      throw new Error("Cannot start an agent session with an empty prompt.");
    }
    targets.push({ agent, prompt: messageBody });
  }

  return { targets, multiRoutedCount: multiRouted.length };
}

export async function fanOutAgentTurns(input: {
  get: WorkspaceGet;
  set: WorkspaceSet;
  conversationId: string;
  workspacePath: string;
  messageBody: string;
  originMessageId: string;
  targets: DispatchTarget[];
  multiRoutedCount: number;
}): Promise<string[]> {
  const {
    get,
    set,
    conversationId,
    workspacePath,
    messageBody,
    originMessageId,
    targets,
    multiRoutedCount,
  } = input;

  await ensureSessionsForRouting(get, conversationId);

  const fanoutResults = await Promise.allSettled(
    targets.map(async (target) => {
      const sessions = get().sessionsByConversation[conversationId] ?? [];
      let sessionId: string | undefined;
      if (target.sessionShortId) {
        const match = sessions.find(
          (s) =>
            s.agent === target.agent &&
            s.status !== "done" &&
            (s.shortId === target.sessionShortId ||
              s.id.endsWith(target.sessionShortId!) ||
              s.id.startsWith(target.sessionShortId!)),
        );
        if (!match) {
          throw new Error(
            `No existing ${target.agent} session matches #${target.sessionShortId}`,
          );
        }
        sessionId = match.id;
      } else if (target.profileId) {
        sessionId = await startNewAgentSession(
          conversationId,
          target.agent,
          workspacePath,
          target.profileId,
        );
      } else {
        const reusable = sessions
          .filter(
            (s) =>
              s.agent === target.agent &&
              !s.parentId &&
              s.status !== "done" &&
              s.status !== "failed",
          )
          .sort((a, b) => (b.lastTsMs ?? 0) - (a.lastTsMs ?? 0))[0];
        sessionId = reusable
          ? reusable.id
          : await startNewAgentSession(
              conversationId,
              target.agent,
              workspacePath,
            );
      }

      const prompt =
        multiRoutedCount > 0
          ? messageBody
          : target.prompt.trim() || messageBody;
      if (!prompt.trim()) {
        throw new Error(`Empty prompt for @${target.agent}`);
      }

      try {
        await daemonApi.resumeSession(sessionId, false);
        set((s) => {
          const prev = s.sessionsById[sessionId!];
          const row = findSessionRow(s, sessionId!);
          const entity = patchSessionEntity(prev, sessionId!, {
            daemonStatus: "running",
            needsContinue: false,
            conversationId:
              prev?.conversationId || row?.conversationId || conversationId,
            agent: prev?.agent || row?.agent,
            shortId: prev?.shortId || row?.shortId,
            model: prev?.model || row?.model,
            summary: prev?.summary || row?.summary,
            parentId: prev?.parentId ?? row?.parentId,
            lastTsMs: Date.now(),
          });
          return commitSessionEntity(s, entity);
        });
      } catch {
        /* not needed when already live */
      }
      await daemonApi.sendUserMessage(sessionId, prompt, originMessageId);
    }),
  );

  return fanoutResults
    .filter((r): r is PromiseRejectedResult => r.status === "rejected")
    .map((r) =>
      r.reason instanceof Error ? r.reason.message : String(r.reason),
    );
}
