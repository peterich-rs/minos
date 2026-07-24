import type { ReactionGroup } from "./reactions";
import { ME_ACTOR } from "./reactions";

/**
 * Mock reaction fixtures for demo timeline messages.
 * Not persisted; reaction-store owns live toggles after seed.
 */
export function seedReactionsByMessageId(): Record<string, ReactionGroup[]> {
  return {
    m2: [
      {
        emoji: "👍",
        count: 2,
        reactedByMe: true,
        actors: [ME_ACTOR, { id: "u-alice", displayName: "Alice" }],
      },
      {
        emoji: "🎉",
        count: 1,
        reactedByMe: false,
        actors: [{ id: "u-bob", displayName: "Bob" }],
      },
    ],
    m6: [
      {
        emoji: "👀",
        count: 1,
        reactedByMe: false,
        actors: [{ id: "u-alice", displayName: "Alice" }],
      },
    ],
    d2: [
      {
        emoji: "❤️",
        count: 1,
        reactedByMe: true,
        actors: [ME_ACTOR],
      },
    ],
    i2: [
      {
        emoji: "👍",
        count: 3,
        reactedByMe: false,
        actors: [
          { id: "u-alice", displayName: "Alice" },
          { id: "u-bob", displayName: "Bob" },
          { id: "u-cara", displayName: "Cara" },
        ],
      },
    ],
  };
}
