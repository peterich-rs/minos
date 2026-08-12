/**
 * Desktop message reactions.
 *
 * - Hub IM mode + Hub message ids → cloud `POST …/reactions/toggle` only
 * - Local workbench / local message ids → daemon LocalReaction* path
 * Never dual-write Hub + daemon for the same bubble.
 */
import { create } from "zustand";
import { daemonApi, isTauriRuntime } from "@/shared/lib/daemon";
import { toast } from "@/shared/lib/toast";
import { isCloudImMode } from "@/shared/lib/cloud-timeline";
import { syncReactionToggleToCloud } from "@/store/im/im-cloud-sync";
import { useAccountStore } from "@/store/account-store";
import {
  hasInFlightToggleCount,
  hydrateReactionsFromMessages,
  mapDaemonReactionGroup,
  shouldApplyToggleResponse,
  toggleReactionGroup,
  type DaemonReactionGroupLike,
  type ReactionGroup,
} from "./lib/reactions";
import { seedReactionsByMessageId } from "./lib/reaction-seed";

function newReactionClientOpId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return `react-${crypto.randomUUID()}`;
  }
  return `react-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

type ReactionState = {
  /**
   * When true, store is backed by local daemon (no mock seed as sole truth).
   * Browser Vite mock mode keeps seed fixtures.
   */
  durableMode: boolean;
  /** Optimistic + durable reaction groups keyed by timeline message id. */
  reactionsByMessageId: Record<string, ReactionGroup[]>;
  /** Monotonic per-message toggle generation (latest wins apply/rollback). */
  toggleGenByMessageId: Record<string, number>;
  /** Outstanding toggle RPCs per message (live/hydrate must not clobber). */
  inFlightCountByMessageId: Record<string, number>;
  /** Clear seed and switch to daemon as source of truth. */
  enterDurableMode: () => void;
  /**
   * Workspace-boundary reset.
   * - `durable-empty`: daemon path (empty maps, durable on)
   * - `mock-seed`: browser preview fixtures
   */
  resetForWorkspaceBoundary: (mode: "durable-empty" | "mock-seed") => void;
  /** True while a durable toggle RPC is in flight for this message. */
  hasInFlightToggle: (messageId: string) => boolean;
  /**
   * Merge reaction snapshots from listed conversation messages.
   * Call after timeline load / quiet re-list / load-older.
   * Skips messages with in-flight toggles so optimistic UI is preserved.
   */
  hydrateFromMessages: (
    messages: ReadonlyArray<{
      id: string;
      reactions?: DaemonReactionGroupLike[] | ReactionGroup[];
    }>,
  ) => void;
  /**
   * Replace groups for a message (server snapshot / live event).
   * No-ops when a toggle is in flight for that message (unless `force`).
   */
  applyServerReactions: (
    messageId: string,
    groups: ReactionGroup[] | DaemonReactionGroupLike[],
    opts?: { force?: boolean },
  ) => void;
  /**
   * Toggle emoji for the current user.
   * Hub IM mode → Hub toggle (aggregate SSOT); else local daemon when durable.
   * Apply/rollback are generation-gated so only the latest toggle wins.
   */
  toggleReaction: (
    messageId: string,
    emoji: string,
    conversationId?: string,
  ) => void;
  /** Replace groups for a message (e.g. hydrate from fixture). */
  setReactions: (messageId: string, groups: ReactionGroup[]) => void;
  /** Read groups; empty array when none. */
  getReactions: (messageId: string) => ReactionGroup[];
};

function toUiGroups(
  groups: ReactionGroup[] | DaemonReactionGroupLike[],
): ReactionGroup[] {
  if (groups.length === 0) return [];
  const first = groups[0] as ReactionGroup | DaemonReactionGroupLike;
  if (
    "actors" in first &&
    Array.isArray(first.actors) &&
    first.actors[0] &&
    "id" in (first.actors[0] as object)
  ) {
    return groups as ReactionGroup[];
  }
  return (groups as DaemonReactionGroupLike[]).map(mapDaemonReactionGroup);
}

function writeGroups(
  reactionsByMessageId: Record<string, ReactionGroup[]>,
  messageId: string,
  groups: ReactionGroup[],
): Record<string, ReactionGroup[]> {
  const next = { ...reactionsByMessageId };
  if (groups.length === 0) {
    delete next[messageId];
  } else {
    next[messageId] = groups;
  }
  return next;
}

export const useReactionStore = create<ReactionState>((set, get) => ({
  durableMode: false,
  reactionsByMessageId: seedReactionsByMessageId(),
  toggleGenByMessageId: {},
  inFlightCountByMessageId: {},

  enterDurableMode: () => {
    get().resetForWorkspaceBoundary("durable-empty");
  },

  resetForWorkspaceBoundary: (mode) => {
    if (mode === "mock-seed") {
      set({
        durableMode: false,
        reactionsByMessageId: seedReactionsByMessageId(),
        toggleGenByMessageId: {},
        inFlightCountByMessageId: {},
      });
      return;
    }
    set({
      durableMode: true,
      reactionsByMessageId: {},
      toggleGenByMessageId: {},
      inFlightCountByMessageId: {},
    });
  },

  hasInFlightToggle: (messageId) =>
    hasInFlightToggleCount(get().inFlightCountByMessageId[messageId]),

  hydrateFromMessages: (messages) => {
    const inflight = get().inFlightCountByMessageId;
    const skipMessageIds = new Set(
      Object.keys(inflight).filter((id) => hasInFlightToggleCount(inflight[id])),
    );
    set((s) => ({
      reactionsByMessageId: hydrateReactionsFromMessages(
        s.reactionsByMessageId,
        messages,
        { skipMessageIds },
      ),
    }));
  },

  applyServerReactions: (messageId, groups, opts) => {
    if (!opts?.force && get().hasInFlightToggle(messageId)) {
      return;
    }
    const ui = toUiGroups(groups);
    set((s) => ({
      reactionsByMessageId: writeGroups(s.reactionsByMessageId, messageId, ui),
    }));
  },

  toggleReaction: (messageId, emoji, conversationId) => {
    const prev = get().getReactions(messageId);
    const optimistic = toggleReactionGroup(prev, emoji);

    const { session, authPhase } = useAccountStore.getState();
    const cloudMode = isCloudImMode({
      authPhase,
      accessToken: session?.accessToken,
    });
    // Hub message ids are typically UUIDs from cloud; prefer Hub when auth + cid.
    const useCloud =
      cloudMode &&
      Boolean(session?.accessToken) &&
      Boolean(conversationId?.trim());

    let requestGen = 0;
    set((s) => {
      requestGen = (s.toggleGenByMessageId[messageId] ?? 0) + 1;
      const toggleGenByMessageId = {
        ...s.toggleGenByMessageId,
        [messageId]: requestGen,
      };
      const inFlightCountByMessageId = { ...s.inFlightCountByMessageId };
      if (useCloud || (s.durableMode && isTauriRuntime())) {
        inFlightCountByMessageId[messageId] =
          (inFlightCountByMessageId[messageId] ?? 0) + 1;
      }
      return {
        toggleGenByMessageId,
        inFlightCountByMessageId,
        reactionsByMessageId: writeGroups(
          s.reactionsByMessageId,
          messageId,
          optimistic,
        ),
      };
    });

    if (!useCloud && (!get().durableMode || !isTauriRuntime())) {
      return;
    }

    void (async () => {
      try {
        if (useCloud && conversationId && session?.accessToken) {
          // Single path: enqueue → flush via outbox (same machine as
          // user_message). No parallel inline POST.
          const clientOpId = newReactionClientOpId();
          try {
            const result = await syncReactionToggleToCloud({
              conversationId,
              messageId,
              emoji,
              clientOpId,
            });
            const currentGen = get().toggleGenByMessageId[messageId] ?? 0;
            if (
              result &&
              shouldApplyToggleResponse(requestGen, currentGen)
            ) {
              // Aggregate only — ignore `action` for state.
              get().applyServerReactions(
                messageId,
                result.reactions.map((g) => ({
                  emoji: g.emoji,
                  count: g.count,
                  reactedByMe: g.reactedByMe,
                  actors: g.actors.map((a) => ({
                    id: a.actorId,
                    displayName: a.displayName,
                  })),
                })),
                { force: true },
              );
            }
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            // Transient: leave optimistic UI; worker retries same client_op_id.
            // Permanent: roll back if still latest gen.
            const permanent =
              /invalid|forbidden|unauthorized|not found|http 4|status: 4|\b4\d\d\b/i.test(
                msg,
              ) && !/408|429|timeout|too many/i.test(msg);
            if (permanent) {
              const currentGen = get().toggleGenByMessageId[messageId] ?? 0;
              if (shouldApplyToggleResponse(requestGen, currentGen)) {
                get().applyServerReactions(messageId, prev, { force: true });
                toast.error("Couldn't update reaction", msg);
              }
            }
          }
        } else {
          const result = await daemonApi.toggleMessageReaction(
            messageId,
            emoji,
          );
          const currentGen = get().toggleGenByMessageId[messageId] ?? 0;
          if (shouldApplyToggleResponse(requestGen, currentGen)) {
            get().applyServerReactions(messageId, result.reactions ?? [], {
              force: true,
            });
          }
        }
      } catch (e) {
        const currentGen = get().toggleGenByMessageId[messageId] ?? 0;
        if (shouldApplyToggleResponse(requestGen, currentGen)) {
          get().applyServerReactions(messageId, prev, { force: true });
          const message = e instanceof Error ? e.message : String(e);
          toast.error("Couldn't update reaction", message);
        }
      } finally {
        set((s) => {
          const inFlightCountByMessageId = { ...s.inFlightCountByMessageId };
          const n = (inFlightCountByMessageId[messageId] ?? 1) - 1;
          if (n <= 0) {
            delete inFlightCountByMessageId[messageId];
          } else {
            inFlightCountByMessageId[messageId] = n;
          }
          return { inFlightCountByMessageId };
        });
      }
    })();
  },

  setReactions: (messageId, groups) => {
    set((s) => ({
      reactionsByMessageId: writeGroups(
        s.reactionsByMessageId,
        messageId,
        groups,
      ),
    }));
  },

  getReactions: (messageId) => get().reactionsByMessageId[messageId] ?? [],
}));
