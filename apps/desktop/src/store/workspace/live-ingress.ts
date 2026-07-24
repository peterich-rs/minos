/**
 * L5 LiveIngress — apply push frames.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import {
  coerceUiSessionStatus,
  conversationRefreshTimers,
  mergeTranscriptItems,
} from "./helpers";
import { commitSessionEntity, findSessionRow } from "./projection";
import {
  applyManagerLifecycleToEntity,
  patchSessionEntity,
} from "@/shared/lib/session-entity";
import {
  demoteResolvedApprovalItems,
  transcriptHasPendingApproval,
} from "@/shared/lib/session-status";
import {
  EMPTY_TRANSCRIPT_HISTORY,
  hasTranscriptWorkingSet,
  trimTranscriptHardMax,
} from "@/shared/lib/transcript-history";
import { hasTimelineWorkingSet } from "@/shared/lib/message-history";
import type { TranscriptItem } from "@/shared/lib/daemon";
import { useReactionStore } from "@/features/chat/reaction-store";


export function createLiveIngressActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "applyIngestEvent"
  | "applyManagerEvent"
  | "applyConversationEvent"
> {
  return {
  applyIngestEvent: (ev) => {
    if (!ev.sessionId) return;
    set((s) => {
      // Thin global ingest: always upsert Entity from the frame flag; only
      // merge heavy transcript items when a Transcript working-set key already
      // exists. Do NOT `?? []` create windows for background sessions.
      const hasTranscriptEntry = hasTranscriptWorkingSet(
        s.transcriptsBySession,
        ev.sessionId,
      );
      let items: TranscriptItem[] | null = null;
      if (hasTranscriptEntry) {
        const merged = mergeTranscriptItems(
          s.transcriptsBySession[ev.sessionId] ?? [],
          ev.items ?? [],
        );
        const trimmed = trimTranscriptHardMax(merged);
        // Never trust a single-frame hasPendingApproval when the window already
        // contains later progress past an answered plan/permission card.
        items = demoteResolvedApprovalItems(trimmed.items);
      }
      const hasPending =
        items != null
          ? transcriptHasPendingApproval(items)
          : Boolean(ev.hasPendingApproval);

      const prevEntity = s.sessionsById[ev.sessionId];
      const known = findSessionRow(s, ev.sessionId);
      // Keep last_seq (messageCount) elevated from live frames so transcript
      // tail seek does not open a stale mid-window when list hydrate lagged.
      const nextMessageCount = Math.max(
        prevEntity?.messageCount ?? 0,
        known?.messageCount ?? 0,
        typeof ev.seq === "number" ? ev.seq : 0,
      );
      // Working-set window (after demote) is high-confidence — may demote.
      // Without a window, only elevate from the frame flag; never clear.
      const nextPending =
        items != null
          ? hasPending
          : hasPending
            ? true
            : (prevEntity?.hasPendingApproval ?? false);
      const entity = patchSessionEntity(prevEntity, ev.sessionId, {
        hasPendingApproval: nextPending,
        daemonStatus: prevEntity?.daemonStatus ?? known?.status ?? "running",
        conversationId:
          known?.conversationId ?? prevEntity?.conversationId ?? "",
        agent: known?.agent ?? prevEntity?.agent ?? ev.agent ?? "codex",
        shortId: known?.shortId ?? prevEntity?.shortId,
        model: known?.model ?? prevEntity?.model,
        summary: known?.summary ?? prevEntity?.summary,
        lastTsMs: ev.tsMs || prevEntity?.lastTsMs,
        messageCount: nextMessageCount,
      });
      const committed = commitSessionEntity(s, entity);

      let transcriptHistoryBySession = s.transcriptHistoryBySession;
      if (items != null) {
        const prevH =
          s.transcriptHistoryBySession[ev.sessionId] ?? EMPTY_TRANSCRIPT_HISTORY;
        const wasTrimmed =
          (s.transcriptsBySession[ev.sessionId]?.length ?? 0) > items.length;
        if (wasTrimmed && !prevH.hasOlder) {
          transcriptHistoryBySession = {
            ...s.transcriptHistoryBySession,
            [ev.sessionId]: { ...prevH, hasOlder: true },
          };
        }
      }

      return {
        ...committed,
        ...(items != null
          ? {
              transcriptsBySession: {
                ...s.transcriptsBySession,
                [ev.sessionId]: items,
              },
              transcriptHistoryBySession,
            }
          : {}),
      };
    });
  },

  applyManagerEvent: (ev) => {
    if (ev.kind === "sessionStateChanged") {
      const status = coerceUiSessionStatus(ev.status);
      set((s) => {
        // Lifecycle only — hasPendingApproval prevents demote of needs_approval
        // while daemon still reports running (Grok park on permission/plan).
        const entity = applyManagerLifecycleToEntity(
          s.sessionsById[ev.sessionId],
          ev.sessionId,
          status,
          { lastTsMs: ev.atMs || undefined },
        );
        return commitSessionEntity(s, entity, { elevateApprovalCount: false });
      });
      return;
    }
    if (ev.kind === "sessionClosed") {
      set((s) => {
        const entity = patchSessionEntity(
          s.sessionsById[ev.sessionId],
          ev.sessionId,
          {
            daemonStatus: "done",
            hasPendingApproval: false,
          },
        );
        return commitSessionEntity(s, entity, { elevateApprovalCount: false });
      });
      return;
    }
    if (ev.kind === "instanceCrashed") {
      const ids = new Set(ev.affectedSessionIds ?? []);
      set((s) => {
        let sessionsById = { ...s.sessionsById };
        let sessionsByConversation = s.sessionsByConversation;
        let projectSessionsByProject = s.projectSessionsByProject;
        let attentionSessions = s.attentionSessions;
        let conversations = s.conversations;
        const attentionStatus = s.attentionStatus;
        for (const id of ids) {
          const entity = patchSessionEntity(sessionsById[id], id, {
            daemonStatus: "suspended",
          });
          const committed = commitSessionEntity(
            {
              sessionsById,
              sessionsByConversation,
              projectSessionsByProject,
              attentionSessions,
              conversations,
              attentionStatus,
            },
            entity,
            { elevateApprovalCount: false },
          );
          sessionsById = committed.sessionsById;
          sessionsByConversation = committed.sessionsByConversation;
          projectSessionsByProject = committed.projectSessionsByProject;
          attentionSessions = committed.attentionSessions;
          conversations = committed.conversations;
        }
        return {
          sessionsById,
          sessionsByConversation,
          projectSessionsByProject,
          attentionSessions,
          conversations,
        };
      });
      return;
    }
    if (ev.kind === "sessionAdded") {
      // Entity shell + quiet refresh any project SessionLists we already hold
      // so Sessions tab does not stay stale while keep-alive / livePush=true.
      if (ev.sessionId) {
        set((s) => {
          if (s.sessionsById[ev.sessionId]) return {};
          const entity = patchSessionEntity(undefined, ev.sessionId, {
            daemonStatus: "idle",
            agent: ev.agent || "codex",
            parentId: ev.parentSessionId ?? undefined,
          });
          return {
            sessionsById: {
              ...s.sessionsById,
              [ev.sessionId]: entity,
            },
          };
        });
        const projectIds = Object.keys(get().projectSessionsByProject);
        for (const pid of projectIds) {
          if ((get().projectSessionsByProject[pid] ?? []).length === 0) continue;
          void get().loadProjectSessions(pid, { quiet: true });
        }
      }
      return;
    }
  },

  applyConversationEvent: (ev) => {
    if (!ev?.conversationId) return;

    // Reaction toggles carry a full aggregate — apply without re-list.
    if (ev.kind === "reactionToggled") {
      useReactionStore
        .getState()
        .applyServerReactions(ev.messageId, ev.reactions ?? []);
      return;
    }

    // Message append: debounced quiet re-list of chat_messages only.
    // Without a Timeline working set: mark dirty, zero RPC.
    const id = ev.conversationId;
    const existing = conversationRefreshTimers.get(id);
    if (existing) clearTimeout(existing);
    conversationRefreshTimers.set(
      id,
      setTimeout(() => {
        conversationRefreshTimers.delete(id);
        const st = get();
        // Always refresh conversation list preview / messageCount for the
        // owning project so the rail does not stay stale under livePush.
        const projectId = st.conversations.find((c) => c.id === id)?.projectId;
        if (projectId) {
          void get().loadConversations(projectId, { quiet: true });
        }
        if (
          !hasTimelineWorkingSet(st.messagesByConversation, id, {
            messageHistoryByConversation: st.messageHistoryByConversation,
            timelineStatusByConversation: st.timelineStatusByConversation,
          })
        ) {
          set((s) => ({
            timelineDirtyByConversation: {
              ...s.timelineDirtyByConversation,
              [id]: true,
            },
          }));
          return;
        }
        void get().loadTimeline(id, { quiet: true });
      }, 200),
    );
  },

  };
}
