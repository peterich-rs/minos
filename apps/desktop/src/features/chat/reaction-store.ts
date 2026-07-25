import { create } from "zustand";
import { daemonApi, isTauriRuntime } from "@/shared/lib/daemon";
import { toast } from "@/shared/lib/toast";
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
   * Optimistic update; when durableMode + Tauri, persists via daemon RPC
   * and replaces with server groups (rollback + toast on failure).
   * Apply/rollback are generation-gated so only the latest toggle wins.
   */
  toggleReaction: (messageId: string, emoji: string) => void;
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

  toggleReaction: (messageId, emoji) => {
    const prev = get().getReactions(messageId);
    const optimistic = toggleReactionGroup(prev, emoji);

    let requestGen = 0;
    set((s) => {
      requestGen = (s.toggleGenByMessageId[messageId] ?? 0) + 1;
      const toggleGenByMessageId = {
        ...s.toggleGenByMessageId,
        [messageId]: requestGen,
      };
      const inFlightCountByMessageId = { ...s.inFlightCountByMessageId };
      if (s.durableMode && isTauriRuntime()) {
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

    if (!get().durableMode || !isTauriRuntime()) {
      return;
    }

    void (async () => {
      try {
        const result = await daemonApi.toggleMessageReaction(messageId, emoji);
        const currentGen = get().toggleGenByMessageId[messageId] ?? 0;
        if (shouldApplyToggleResponse(requestGen, currentGen)) {
          get().applyServerReactions(messageId, result.reactions ?? [], {
            force: true,
          });
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
