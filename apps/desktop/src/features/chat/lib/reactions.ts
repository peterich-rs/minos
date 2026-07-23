/** Domain-neutral message reaction types + pure toggle merge. */

export type ReactionActor = {
  id: string;
  displayName: string;
};

export type ReactionGroup = {
  /** Unicode emoji, e.g. "👍". */
  emoji: string;
  count: number;
  reactedByMe: boolean;
  /** Sample actors for tooltip (may be a subset when count is large). */
  actors: ReactionActor[];
};

/**
 * Wire DTO shape from daemon (camelCase). Kept local so reactions.ts stays
 * free of the daemon module for pure unit tests.
 */
export type DaemonReactionGroupLike = {
  emoji: string;
  count: number;
  reactedByMe: boolean;
  actors?: Array<{
    actorId: string;
    actorKind: string;
    displayName: string;
  }>;
};

/** Local "me" actor for optimistic toggles (mock / desktop host user). */
export const ME_ACTOR: ReactionActor = {
  id: "me",
  displayName: "You",
};

/** Daemon host actor id (single local user). */
export const LOCAL_DAEMON_ACTOR_ID = "local";

/** Quick-react strip shown before opening the full picker. */
export const QUICK_REACTION_EMOJIS = ["👍", "❤️", "😂", "🎉", "👀"] as const;

/**
 * Toggle `emoji` for the current user on a message's reaction groups.
 * Pure: returns a new array; empty result means no reactions remain.
 */
export function toggleReactionGroup(
  groups: readonly ReactionGroup[] | undefined,
  emoji: string,
  me: ReactionActor = ME_ACTOR,
): ReactionGroup[] {
  const next = (groups ?? []).map((g) => ({
    ...g,
    actors: [...g.actors],
  }));
  const idx = next.findIndex((g) => g.emoji === emoji);

  if (idx === -1) {
    next.push({
      emoji,
      count: 1,
      reactedByMe: true,
      actors: [me],
    });
    return sortReactionGroups(next);
  }

  const group = next[idx]!;
  if (group.reactedByMe) {
    // actors is a sample subset — only drop the group when count hits 0.
    const actors = group.actors.filter((a) => a.id !== me.id);
    const count = Math.max(0, group.count - 1);
    if (count === 0) {
      next.splice(idx, 1);
    } else {
      next[idx] = {
        ...group,
        count,
        reactedByMe: false,
        actors,
      };
    }
  } else {
    const alreadyListed = group.actors.some((a) => a.id === me.id);
    next[idx] = {
      ...group,
      count: group.count + 1,
      reactedByMe: true,
      actors: alreadyListed ? group.actors : [...group.actors, me],
    };
  }

  return sortReactionGroups(next);
}

/** Stable display order: higher count first, then emoji codepoint. */
export function sortReactionGroups(groups: ReactionGroup[]): ReactionGroup[] {
  return [...groups].sort((a, b) => {
    if (b.count !== a.count) return b.count - a.count;
    return a.emoji.localeCompare(b.emoji);
  });
}

/** Tooltip label: "You, Alice, Bob" or "You and 3 others". */
export function reactionActorsLabel(group: ReactionGroup): string {
  if (group.actors.length === 0) {
    return `${group.count} reaction${group.count === 1 ? "" : "s"}`;
  }
  const names = group.actors.map((a) =>
    a.id === ME_ACTOR.id ? "You" : a.displayName,
  );
  if (group.count <= names.length) {
    if (names.length === 1) return names[0]!;
    if (names.length === 2) return `${names[0]} and ${names[1]}`;
    return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
  }
  const extra = group.count - names.length;
  if (names.length === 1) return `${names[0]} and ${extra} other${extra === 1 ? "" : "s"}`;
  return `${names.join(", ")} and ${extra} other${extra === 1 ? "" : "s"}`;
}

/** Map daemon actor id `local` → client `ME_ACTOR`. */
export function mapDaemonActorId(actorId: string): string {
  return actorId === LOCAL_DAEMON_ACTOR_ID ? ME_ACTOR.id : actorId;
}

/** Map a daemon reaction group into UI ReactionGroup (local → me). */
export function mapDaemonReactionGroup(
  group: DaemonReactionGroupLike,
): ReactionGroup {
  const actors = (group.actors ?? []).map((a) => {
    const id = mapDaemonActorId(a.actorId);
    return {
      id,
      displayName: id === ME_ACTOR.id ? ME_ACTOR.displayName : a.displayName,
    };
  });
  return {
    emoji: group.emoji,
    count: group.count,
    reactedByMe: group.reactedByMe,
    actors,
  };
}

function isUiReactionGroup(g: unknown): g is ReactionGroup {
  if (!g || typeof g !== "object") return false;
  const actors = (g as ReactionGroup).actors;
  if (!Array.isArray(actors)) return false;
  if (actors.length === 0) {
    return (
      typeof (g as ReactionGroup).emoji === "string" &&
      typeof (g as ReactionGroup).count === "number" &&
      typeof (g as ReactionGroup).reactedByMe === "boolean"
    );
  }
  return typeof actors[0]?.id === "string";
}

/**
 * Whether a completed toggle RPC should update UI for `messageId`.
 * Only the latest generation's response (success or failure) may apply.
 */
export function shouldApplyToggleResponse(
  requestGen: number,
  currentGen: number,
): boolean {
  return requestGen === currentGen;
}

/** True when at least one toggle RPC is still outstanding for a message. */
export function hasInFlightToggleCount(count: number | undefined): boolean {
  return (count ?? 0) > 0;
}

/**
 * Merge daemon message reaction snapshots into a by-message map.
 * Messages with empty/missing reactions remove any prior entry for that id
 * (durable hydrate wins over seed/optimistic for listed messages).
 *
 * `skipMessageIds` keeps optimistic state for messages with in-flight toggles.
 */
export function hydrateReactionsFromMessages(
  prev: Record<string, ReactionGroup[]>,
  messages: ReadonlyArray<{
    id: string;
    reactions?: DaemonReactionGroupLike[] | ReactionGroup[];
  }>,
  options?: { skipMessageIds?: ReadonlySet<string> },
): Record<string, ReactionGroup[]> {
  if (messages.length === 0) return prev;
  const next = { ...prev };
  const skip = options?.skipMessageIds;
  for (const m of messages) {
    if (skip?.has(m.id)) continue;
    const raw = m.reactions ?? [];
    if (raw.length === 0) {
      delete next[m.id];
      continue;
    }
    next[m.id] = raw.map((g) =>
      isUiReactionGroup(g)
        ? g
        : mapDaemonReactionGroup(g as DaemonReactionGroupLike),
    );
  }
  return next;
}
