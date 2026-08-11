/**
 * L3b Transcript window + resume interrupted session.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import {
  dedupeTranscriptItemsById,
  mergeTranscriptItems,
  statusForLoad,
} from "./helpers";
import { commitSessionEntity, findSessionRow } from "./projection";
import {
  daemonStatusFromEntity,
  mergeSessionEntity,
  patchSessionEntity,
} from "@/shared/lib/session-entity";
import { daemonApi } from "@/shared/lib/daemon";
import {
  resumeInFlightSessions,
  resumedInterruptedSessions,
  singleFlightLoad,
} from "@/shared/lib/desktop-inflight";
import {
  demoteResolvedApprovalItems,
  transcriptHasPendingApproval,
  type ApprovalStatusPolicy,
} from "@/shared/lib/session-status";
import {
  EMPTY_TRANSCRIPT_HISTORY,
  mergeTranscriptOlder,
  metaAfterTailLoad,
  olderPageRange,
  tailFromSeq,
  TRANSCRIPT_PAGE_EVENTS,
  trimTranscriptHardMax,
  hasTranscriptWorkingSet,
} from "@/shared/lib/transcript-history";
import type { SessionStatus } from "@/shared/lib/mock-data";


export function createTranscriptActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "loadTranscript"
  | "resumeInterruptedSession"
> {
  return {
  loadTranscript: async (sessionId, opts) => {
    // Mock: no transcript RPC; do not arm loads against daemon.
    if (get().source !== "daemon" || !sessionId) return;

    const older = opts?.older === true;
    const append = opts?.append === true && !older;
    const full = opts?.full === true && !older && !append;
    // Older pages and appends never flash the main loading skeleton.
    const quiet = opts?.quiet === true || append || older;

    const approvalStatusPolicy: ApprovalStatusPolicy =
      opts?.approvalStatusPolicy ??
      (append || older ? "sync" : "elevate-only");

    const modeKey = older
      ? "older"
      : append
        ? "append"
        : full
          ? "full"
          : quiet
            ? "q"
            : "h";

    return singleFlightLoad(`transcript:${sessionId}:${modeKey}`, async () => {
      // ── Older page (infinite scroll) ───────────────────────────────
      if (older) {
        const hist =
          get().transcriptHistoryBySession[sessionId] ??
          EMPTY_TRANSCRIPT_HISTORY;
        if (hist.loadingOlder || !hist.hasOlder) return;
        const range = olderPageRange(
          hist.firstLoadedStartSeq,
          opts?.tailWindow ?? TRANSCRIPT_PAGE_EVENTS,
        );
        if (!range) {
          set((s) => ({
            transcriptHistoryBySession: {
              ...s.transcriptHistoryBySession,
              [sessionId]: {
                firstLoadedStartSeq: 1,
                hasOlder: false,
                loadingOlder: false,
              },
            },
          }));
          return;
        }

        set((s) => ({
          transcriptHistoryBySession: {
            ...s.transcriptHistoryBySession,
            [sessionId]: {
              firstLoadedStartSeq: hist.firstLoadedStartSeq,
              hasOlder: hist.hasOlder,
              loadingOlder: true,
            },
          },
        }));

        try {
          const page = await daemonApi.readTranscript(
            sessionId,
            range.fromSeq,
            range.limit,
          );
          if (get().source !== "daemon") return;
          set((s) => {
            const existing = s.transcriptsBySession[sessionId] ?? [];
            let items = mergeTranscriptOlder(page.items, existing);
            const trimmed = trimTranscriptHardMax(items);
            items = trimmed.items;
            const nextFirst = range.nextFirstLoadedStartSeq;
            return {
              transcriptsBySession: {
                ...s.transcriptsBySession,
                [sessionId]: items,
              },
              transcriptHistoryBySession: {
                ...s.transcriptHistoryBySession,
                [sessionId]: {
                  firstLoadedStartSeq: nextFirst,
                  hasOlder: nextFirst > 1 || trimmed.trimmed,
                  loadingOlder: false,
                },
              },
            };
          });
        } catch {
          set((s) => {
            const prevH =
              s.transcriptHistoryBySession[sessionId] ??
              EMPTY_TRANSCRIPT_HISTORY;
            return {
              transcriptHistoryBySession: {
                ...s.transcriptHistoryBySession,
                [sessionId]: {
                  firstLoadedStartSeq: prevH.firstLoadedStartSeq,
                  hasOlder: prevH.hasOlder,
                  loadingOlder: false,
                },
              },
            };
          });
        }
        return;
      }

      // ── Initial / tail / append / full ─────────────────────────────
      // Quiet peeks (inspector elevate-only) must NOT bump generation: a
      // concurrent hard open would otherwise complete as "stale" and discard
      // the full page, leaving an empty working-set window forever under
      // livePush (no append poll to recover).
      const prev = get().transcriptStatusBySession[sessionId];
      const { next, generation } = statusForLoad(prev, quiet);
      // Quiet is stale when a hard open bumped generation past what we saw.
      // Hard open is stale when a newer hard open superseded us.
      const isStale = () =>
        get().transcriptStatusBySession[sessionId]?.generation !== generation;

      set((s) => ({
        transcriptStatusBySession: {
          ...s.transcriptStatusBySession,
          [sessionId]: next,
        },
        // Ensure working-set key exists so ingest can merge (incl. empty).
        transcriptsBySession: hasTranscriptWorkingSet(
          s.transcriptsBySession,
          sessionId,
        )
          ? s.transcriptsBySession
          : { ...s.transcriptsBySession, [sessionId]: [] },
      }));

      try {
        const existing = get().transcriptsBySession[sessionId] ?? [];
        const session = findSessionRow(get(), sessionId);
        const entityMessageCount =
          get().sessionsById[sessionId]?.messageCount ?? 0;

        // Daemon `from_seq` is exclusive: start = from_seq + 1.
        let fromSeq: number | undefined;
        const window = opts?.tailWindow ?? TRANSCRIPT_PAGE_EVENTS;
        const pageLimit = full ? 1000 : Math.max(window, 500);
        if (append && existing.length > 0) {
          fromSeq = Math.max(...existing.map((i) => i.seq));
        } else if (!full) {
          // Prefer live-elevated Entity last_seq over possibly stale list row.
          const lastSeq = Math.max(
            session?.messageCount ?? 0,
            entityMessageCount,
          );
          fromSeq = tailFromSeq(lastSeq, window);
        }

        let page = await daemonApi.readTranscript(
          sessionId,
          fromSeq,
          pageLimit,
          { full },
        );
        if (isStale()) return;

        // Stale last_seq seek can land mid-history (nextSeq set). Catch up to
        // the true end so the open transcript includes latest turns — otherwise
        // the user only sees a middle window and cannot scroll to the real tail.
        // Daemon nextSeq is the next inclusive start; exclusive from = next - 1.
        if (!full && page.nextSeq != null) {
          let guard = 0;
          const maxCatchUp = 50;
          while (page.nextSeq != null && guard < maxCatchUp) {
            guard += 1;
            const more = await daemonApi.readTranscript(
              sessionId,
              page.nextSeq - 1,
              pageLimit,
            );
            if (isStale()) return;
            page = {
              sessionId,
              items: mergeTranscriptItems(page.items, more.items),
              nextSeq: more.nextSeq,
            };
          }
        }

        // Elevate last_seq from the highest item we actually saw.
        const observedMaxSeq =
          page.items.length > 0
            ? Math.max(...page.items.map((i) => i.seq))
            : 0;

        set((s) => {
          // Re-check inside set: a hard open may have finished between await
          // and commit; quiet must never clobber a newer generation's window.
          if (
            s.transcriptStatusBySession[sessionId]?.generation !== generation
          ) {
            return {};
          }

          const prevItems = append
            ? (s.transcriptsBySession[sessionId] ?? [])
            : [];
          // When replacing (non-append peek), merge with prior items so a short
          // tail window cannot drop an already-known pending approval card.
          // Quiet/elevate-only always merges; hard sync replace only when not
          // appending (full open / reopen without cache).
          // Hard open also merges concurrent ingest frames that arrived while
          // the RPC was in flight (same race as timeline hard vs quiet).
          const base = append
            ? prevItems
            : (s.transcriptsBySession[sessionId] ?? []);
          const merged = mergeTranscriptItems(base, page.items);
          let items = dedupeTranscriptItemsById(merged);
          // Quiet peek with empty page must not wipe a non-empty window that
          // was filled by ingest or a concurrent hard open (generation tied).
          if (quiet && items.length === 0 && base.length > 0) {
            items = base;
          }
          const trimmed = trimTranscriptHardMax(items);
          // mergeTranscriptItems already demotes resolved approvals; re-run after
          // hard-max trim so a trimmed window still does not re-elevate pending.
          items = demoteResolvedApprovalItems(trimmed.items);
          const hasPendingApproval = transcriptHasPendingApproval(items);

          // Entity is sole status writer; list projection + aggregates follow.
          // Seed lifecycle from daemonStatus — never UI status (needs_approval
          // would coerce to running and erase idle parks).
          const prevEntity = s.sessionsById[sessionId];
          const elevatedCount = Math.max(
            session?.messageCount ?? 0,
            prevEntity?.messageCount ?? 0,
            observedMaxSeq,
          );
          const seed = {
            id: sessionId,
            conversationId:
              session?.conversationId || prevEntity?.conversationId || "",
            conversationTitle:
              session?.conversationTitle ?? prevEntity?.conversationTitle,
            agent: session?.agent || prevEntity?.agent || "codex",
            shortId:
              session?.shortId ||
              prevEntity?.shortId ||
              sessionId.slice(0, 8),
            status: daemonStatusFromEntity(
              prevEntity,
              (session?.status as SessionStatus | undefined) ?? "running",
            ),
            model: session?.model || prevEntity?.model || "",
            parentId: session?.parentId ?? prevEntity?.parentId,
            summary: session?.summary ?? prevEntity?.summary ?? "",
            messageCount: elevatedCount || session?.messageCount,
            firstTsMs: session?.firstTsMs ?? prevEntity?.firstTsMs,
            lastTsMs: session?.lastTsMs ?? prevEntity?.lastTsMs,
            needsContinue:
              prevEntity?.needsContinue ?? session?.needsContinue,
          };
          const entity = mergeSessionEntity(prevEntity, seed, {
            pendingApproval: hasPendingApproval,
            approvalPolicy: approvalStatusPolicy,
            // Transcript scan is high-confidence for pending; lifecycle stays sample
            // so we do not invent running from UI status.
            lifecycleSource: "sample",
          });
          const committed = commitSessionEntity(s, entity);

          let transcriptHistoryBySession = s.transcriptHistoryBySession
          // Quiet peeks must not reset infinite-scroll cursors for a window
          // that was already opened with a full tail/history.
          if (!append && !quiet) {
            const hist = full
              ? {
                  firstLoadedStartSeq: 1,
                  // After catch-up, nextSeq should be null; trimmed still
                  // signals more history above the hard max window.
                  hasOlder: page.nextSeq != null || trimmed.trimmed,
                  loadingOlder: false,
                }
              : {
                  ...metaAfterTailLoad(fromSeq),
                  hasOlder:
                    metaAfterTailLoad(fromSeq).hasOlder || trimmed.trimmed,
                };
            transcriptHistoryBySession = {
              ...s.transcriptHistoryBySession,
              [sessionId]: hist,
            };
          } else if (trimmed.trimmed) {
            const prevH =
              s.transcriptHistoryBySession[sessionId] ??
              EMPTY_TRANSCRIPT_HISTORY;
            transcriptHistoryBySession = {
              ...s.transcriptHistoryBySession,
              [sessionId]: { ...prevH, hasOlder: true },
            };
          }

          return {
            ...committed,
            transcriptsBySession: {
              ...s.transcriptsBySession,
              [sessionId]: items,
            },
            transcriptStatusBySession: {
              ...s.transcriptStatusBySession,
              [sessionId]: { phase: "ready", generation },
            },
            transcriptHistoryBySession,
          };
        });
      } catch (e) {
        if (isStale()) return;
        // Quiet failures must not flip a healthy open into error (no flash).
        // Still release a quiet-only "loading" phase so the UI is not stuck.
        if (quiet) {
          set((s) => {
            const cur = s.transcriptStatusBySession[sessionId];
            if (cur?.generation !== generation) return {};
            if (cur.phase !== "loading") return {};
            return {
              transcriptStatusBySession: {
                ...s.transcriptStatusBySession,
                [sessionId]: { phase: "ready", generation },
              },
            };
          });
          return;
        }
        const message = e instanceof Error ? e.message : String(e);
        set((s) => ({
          transcriptStatusBySession: {
            ...s.transcriptStatusBySession,
            [sessionId]: { phase: "error", generation, error: message },
          },
        }));
      }
    });
  },

  resumeInterruptedSession: async (sessionId) => {
    if (get().source !== "daemon" || !sessionId) return;
    if (
      resumedInterruptedSessions.has(sessionId) ||
      resumeInFlightSessions.has(sessionId)
    ) {
      return;
    }

    const session = findSessionRow(get(), sessionId);
    if (!session) return;
    // Only auto-continue mid-turn death. Plain suspended/idle reattach happens
    // on the next user send (sendMessage → resume false).
    if (!session.needsContinue) return;

    resumeInFlightSessions.add(sessionId);
    try {
      await daemonApi.resumeSession(sessionId, true);
      resumedInterruptedSessions.add(sessionId);

      // Entity is the sole status SSOT. After auto-continue succeeds, paint
      // Running on every surface that projects Entity (Inspector rail, Session
      // detail, Sessions tab) *before* quiet re-list — list RPC / manager
      // SessionStateChanged can lag, which previously left the rail on Paused
      // while the agent was already working (only Transcript open re-listed).
      set((s) => {
        const prev = s.sessionsById[sessionId];
        const entity = patchSessionEntity(prev, sessionId, {
          daemonStatus: "running",
          needsContinue: false,
          conversationId:
            prev?.conversationId || session.conversationId || "",
          conversationTitle:
            prev?.conversationTitle ?? session.conversationTitle,
          agent: prev?.agent || session.agent,
          shortId: prev?.shortId || session.shortId,
          model: prev?.model || session.model,
          summary: prev?.summary || session.summary,
          parentId: prev?.parentId ?? session.parentId,
          messageCount: prev?.messageCount ?? session.messageCount,
          firstTsMs: prev?.firstTsMs ?? session.firstTsMs,
          lastTsMs: Date.now(),
        });
        return commitSessionEntity(s, entity, {
          elevateApprovalCount: false,
        });
      });

      // Confirm with daemon list (may refine idle/running/approval). Quiet so
      // we do not flash loaders; Entity already shows live.
      const convId =
        get().sessionsById[sessionId]?.conversationId ||
        session.conversationId;
      const projectId = get().conversations.find((c) => c.id === convId)
        ?.projectId;
      if (convId) {
        await get().loadTimeline(convId, { quiet: true });
        const st = get();
        if (
          Object.prototype.hasOwnProperty.call(
            st.sessionsByConversation,
            convId,
          ) ||
          Object.prototype.hasOwnProperty.call(
            st.inspectorStatusByConversation,
            convId,
          )
        ) {
          await get().loadInspector(convId, { quiet: true });
        }
      }
      if (projectId) {
        await get().loadProjectSessions(projectId, { quiet: true });
      }
    } catch (e) {
      // Leave needsContinue set server-side so a later open/send can retry.
      console.warn(
        "[minos] resumeInterruptedSession failed",
        sessionId,
        e instanceof Error ? e.message : e,
      );
    } finally {
      resumeInFlightSessions.delete(sessionId);
    }
  },

  /**
   * Attention page queue (heavy). Badge does NOT use this list — badge is
   * Σ project.needsAttention over ConversationLists for known projects.
   */
  };
}
