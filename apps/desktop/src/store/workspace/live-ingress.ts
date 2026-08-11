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

/**
 * Debounced quiet re-list of conversation timeline (+ rail preview).
 *
 * Shared by conversation message push and session turn-end (idle/done):
 * daemon `conversation_completion` writes local `agent-result:…` (workbench);
 * Linked Hub also needs outbox uplink even when the timeline is not open —
 * otherwise agent bubbles only appear after leave/return hydrate.
 */
function scheduleConversationTimelineRefresh(
  get: WorkspaceGet,
  conversationId: string,
): void {
  if (!conversationId) return;
  const existing = conversationRefreshTimers.get(conversationId);
  if (existing) clearTimeout(existing);
  conversationRefreshTimers.set(
    conversationId,
    setTimeout(() => {
      conversationRefreshTimers.delete(conversationId);
      void runConversationTurnEndRefresh(get, conversationId);
    }, 200),
  );
}

async function runConversationTurnEndRefresh(
  get: WorkspaceGet,
  conversationId: string,
): Promise<void> {
  const st = get();
  const projectId = st.conversations.find(
    (c) => c.id === conversationId,
  )?.projectId;
  const focused = st.focusedConversationId === conversationId;
  const hasWorkingSet = hasTimelineWorkingSet(
    st.messagesByConversation,
    conversationId,
    {
      messageHistoryByConversation: st.messageHistoryByConversation,
      timelineStatusByConversation: st.timelineStatusByConversation,
    },
  );

  // 1) Open timeline first — never gate UI on Hub uplink/network.
  //    conversation_completion already wrote local agent-result; quiet re-list
  //    must surface it immediately while the conversation is open.
  if (focused || hasWorkingSet) {
    void get().loadTimeline(conversationId, { quiet: true });
  }

  // 2) Rail preview/count (single quiet re-list; digest live-patch covers most).
  if (projectId) {
    void get().loadConversations(projectId, { quiet: true });
  }

  // 3) Background Hub uplink of local agent-result rows (even when unopened).
  //    Must not block step 1 — slow/failed outbox used to leave the open chat
  //    empty until leave/return hard hydrate.
  try {
    const { useAccountStore } = await import("@/store/account-store");
    const { isCloudImMode } = await import("@/shared/lib/cloud-timeline");
    const { projectMissingLocalAgentResultsToCloud, flushImOutbox } =
      await import("@/shared/lib/im-cloud-sync");
    const { toUiMessage } = await import("./helpers");
    const { daemonApi } = await import("@/shared/lib/daemon");
    const { session, authPhase } = useAccountStore.getState();
    if (
      !isCloudImMode({
        authPhase,
        accessToken: session?.accessToken,
      })
    ) {
      return;
    }
    const page = await daemonApi.listMessages(conversationId, {
      limit: 50,
    });
    const localUi = (page.messages ?? []).map((m) => toUiMessage(m));
    await projectMissingLocalAgentResultsToCloud(conversationId, localUi, []);
    void flushImOutbox();
    // Open window may still be on local-only agent-result; quiet re-merge after
    // uplink so Hub body/reactions can replace same-id rows without a leave.
    const stAfter = get();
    const stillOpen =
      stAfter.focusedConversationId === conversationId ||
      hasTimelineWorkingSet(stAfter.messagesByConversation, conversationId, {
        messageHistoryByConversation: stAfter.messageHistoryByConversation,
        timelineStatusByConversation: stAfter.timelineStatusByConversation,
      });
    if (stillOpen) {
      void get().loadTimeline(conversationId, { quiet: true });
    }
  } catch (err) {
    console.warn(
      "[live-ingress] turn-end agent-result hub uplink failed",
      err,
    );
  }
}

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
      // Track hard-max trim separately — length can also drop from subagent
      // collapse / demote, which is not "older history exists".
      let hardMaxTrimmed = false;
      if (hasTranscriptEntry) {
        const merged = mergeTranscriptItems(
          s.transcriptsBySession[ev.sessionId] ?? [],
          ev.items ?? [],
        );
        const trimmed = trimTranscriptHardMax(merged);
        hardMaxTrimmed = trimmed.trimmed;
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
        if (hardMaxTrimmed && !prevH.hasOlder) {
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
      const turnEnded =
        status === "idle" || status === "done" || status === "failed";
      let conversationIdForTimeline: string | undefined;
      set((s) => {
        const prev = s.sessionsById[ev.sessionId];
        const known = findSessionRow(s, ev.sessionId);
        // Lifecycle only — hasPendingApproval prevents demote of needs_approval
        // while daemon still reports running (Grok park on permission/plan).
        // Turn-end (idle/done/failed) clears sticky pending: mid-turn tool
        // permission frames can elevate hasPendingApproval without a transcript
        // window, and elevate-only never clears — leaving needs_approval/Running
        // forever even after the daemon is idle.
        let entity = applyManagerLifecycleToEntity(
          prev,
          ev.sessionId,
          status,
          { lastTsMs: ev.atMs || undefined },
        );
        if (turnEnded && entity.hasPendingApproval) {
          entity = patchSessionEntity(entity, ev.sessionId, {
            hasPendingApproval: false,
            daemonStatus: status,
            lastTsMs: entity.lastTsMs,
          });
        }
        // SessionAdded shells often lack conversationId; copy from list hydrate
        // so projection can upsert into the inspector membership list.
        if (!entity.conversationId && known?.conversationId) {
          entity = patchSessionEntity(entity, ev.sessionId, {
            conversationId: known.conversationId,
            conversationTitle: known.conversationTitle,
            agent: known.agent || entity.agent,
            shortId: known.shortId || entity.shortId,
            model: known.model || entity.model,
            summary: known.summary || entity.summary,
            parentId: known.parentId ?? entity.parentId,
            messageCount: known.messageCount ?? entity.messageCount,
            firstTsMs: known.firstTsMs ?? entity.firstTsMs,
            lastTsMs: entity.lastTsMs ?? known.lastTsMs,
          });
        }
        conversationIdForTimeline = entity.conversationId || known?.conversationId;
        return commitSessionEntity(s, entity, { elevateApprovalCount: false });
      });
      // Turn end (idle/done/failed): conversation_completion may have just
      // written agent-result into chat_messages. Conversation push can lag or
      // drop; do not wait for livePush alone — quiet re-list the timeline.
      // Also re-list inspector sessions so membership rows reconcile daemon
      // idle even if an earlier optimistic running patch raced the event.
      if (conversationIdForTimeline && turnEnded) {
        scheduleConversationTimelineRefresh(get, conversationIdForTimeline);
        void get().loadInspector(conversationIdForTimeline, { quiet: true });
      }
      return;
    }
    if (ev.kind === "sessionClosed") {
      let conversationIdForTimeline: string | undefined;
      set((s) => {
        const prev = s.sessionsById[ev.sessionId];
        const known = findSessionRow(s, ev.sessionId);
        let entity = patchSessionEntity(prev, ev.sessionId, {
          daemonStatus: "done",
          hasPendingApproval: false,
        });
        if (!entity.conversationId && known?.conversationId) {
          entity = patchSessionEntity(entity, ev.sessionId, {
            conversationId: known.conversationId,
            conversationTitle: known.conversationTitle,
            agent: known.agent || entity.agent,
            shortId: known.shortId || entity.shortId,
            model: known.model || entity.model,
            summary: known.summary || entity.summary,
            parentId: known.parentId ?? entity.parentId,
          });
        }
        conversationIdForTimeline = entity.conversationId || known?.conversationId;
        return commitSessionEntity(s, entity, { elevateApprovalCount: false });
      });
      if (conversationIdForTimeline) {
        scheduleConversationTimelineRefresh(get, conversationIdForTimeline);
      }
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
      // Entity shell via sole commit (projects membership when conversationId
      // known from parent). Quiet re-list project SessionLists we already hold.
      if (ev.sessionId) {
        set((s) => {
          if (s.sessionsById[ev.sessionId]) return {};
          const parent = ev.parentSessionId
            ? s.sessionsById[ev.parentSessionId]
            : undefined;
          const entity = patchSessionEntity(undefined, ev.sessionId, {
            daemonStatus: "idle",
            agent: ev.agent || "codex",
            parentId: ev.parentSessionId ?? undefined,
            conversationId: parent?.conversationId ?? "",
            conversationTitle: parent?.conversationTitle,
          });
          return commitSessionEntity(s, entity, {
            elevateApprovalCount: false,
          });
        });
        // Quiet re-list every project SessionList we already hold (including
        // single-row membership from live commit upsert) so full history
        // catches up without waiting for the user to re-open Sessions.
        const projectIds = Object.keys(get().projectSessionsByProject);
        for (const pid of projectIds) {
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

    // Message append / roster: debounced quiet re-list of chat_messages.
    // Background (no window): next open loadTimeline cold-pulls; no dirty flag.
    scheduleConversationTimelineRefresh(get, ev.conversationId);
  },

  };
}
