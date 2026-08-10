/**
 * Desktop user-turn dispatch: participant delivery target resolution + fan-out.
 *
 * Aligns with Hub room rules (ADR 0021 / agent-participant-delivery).
 * Empty targets still allow send: append user bubble + Hub client_live uplink;
 * do not start daemon sessions or HostCommand.
 */

import type { WorkspaceGet, WorkspaceSet } from "./types";
import { commitSessionEntity, findSessionRow } from "./projection";
import { patchSessionEntity } from "@/shared/lib/session-entity";
import { daemonApi } from "@/shared/lib/daemon";
import { ensureSessionsForRouting, startNewAgentSession } from "./shared";
import type { DispatchTarget } from "./resolve-dispatch-targets";

export {
  resolveDispatchTargets,
  type DispatchTarget,
  type ResolveDispatchTargetsInput,
} from "./resolve-dispatch-targets";

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

  if (targets.length === 0) return [];

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
