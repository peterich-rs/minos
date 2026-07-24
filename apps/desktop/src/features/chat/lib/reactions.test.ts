import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  hasInFlightToggleCount,
  hydrateReactionsFromMessages,
  mapDaemonReactionGroup,
  ME_ACTOR,
  reactionActorsLabel,
  shouldApplyToggleResponse,
  toggleReactionGroup,
  type ReactionGroup,
} from "./reactions.ts";

function group(
  partial: Partial<ReactionGroup> & Pick<ReactionGroup, "emoji">,
): ReactionGroup {
  return {
    count: partial.count ?? 1,
    reactedByMe: partial.reactedByMe ?? false,
    actors: partial.actors ?? [{ id: "u1", displayName: "Alice" }],
    ...partial,
  };
}

describe("toggleReactionGroup", () => {
  it("adds a new emoji when none present", () => {
    const next = toggleReactionGroup(undefined, "👍");
    assert.equal(next.length, 1);
    assert.equal(next[0]!.emoji, "👍");
    assert.equal(next[0]!.count, 1);
    assert.equal(next[0]!.reactedByMe, true);
    assert.deepEqual(next[0]!.actors, [ME_ACTOR]);
  });

  it("removes my reaction and drops the group when count hits 0", () => {
    const prev = [
      group({
        emoji: "❤️",
        count: 1,
        reactedByMe: true,
        actors: [ME_ACTOR],
      }),
    ];
    const next = toggleReactionGroup(prev, "❤️");
    assert.deepEqual(next, []);
  });

  it("decrements count and keeps others when I unreact", () => {
    const prev = [
      group({
        emoji: "🎉",
        count: 3,
        reactedByMe: true,
        actors: [ME_ACTOR, { id: "u1", displayName: "Alice" }],
      }),
    ];
    const next = toggleReactionGroup(prev, "🎉");
    assert.equal(next.length, 1);
    assert.equal(next[0]!.count, 2);
    assert.equal(next[0]!.reactedByMe, false);
    assert.equal(next[0]!.actors.some((a) => a.id === ME_ACTOR.id), false);
  });

  it("keeps group when unreacting with partial actor sample", () => {
    // Only ME is sampled in actors, but count is 5 (others not listed).
    const prev = [
      group({
        emoji: "👍",
        count: 5,
        reactedByMe: true,
        actors: [ME_ACTOR],
      }),
    ];
    const next = toggleReactionGroup(prev, "👍");
    assert.equal(next.length, 1);
    assert.equal(next[0]!.emoji, "👍");
    assert.equal(next[0]!.count, 4);
    assert.equal(next[0]!.reactedByMe, false);
    assert.deepEqual(next[0]!.actors, []);
  });

  it("increments an existing group when I react", () => {
    const prev = [
      group({
        emoji: "👀",
        count: 2,
        reactedByMe: false,
        actors: [
          { id: "u1", displayName: "Alice" },
          { id: "u2", displayName: "Bob" },
        ],
      }),
    ];
    const next = toggleReactionGroup(prev, "👀");
    assert.equal(next[0]!.count, 3);
    assert.equal(next[0]!.reactedByMe, true);
    assert.ok(next[0]!.actors.some((a) => a.id === ME_ACTOR.id));
  });

  it("does not clear other emoji groups when toggling one", () => {
    const prev = [
      group({ emoji: "👍", count: 1, reactedByMe: true, actors: [ME_ACTOR] }),
      group({
        emoji: "😂",
        count: 1,
        reactedByMe: false,
        actors: [{ id: "u1", displayName: "Alice" }],
      }),
    ];
    const next = toggleReactionGroup(prev, "👍");
    assert.equal(next.length, 1);
    assert.equal(next[0]!.emoji, "😂");
  });

  it("sorts by count desc then emoji", () => {
    const next = toggleReactionGroup(
      [group({ emoji: "👍", count: 1, reactedByMe: false })],
      "😂",
    );
    // both count 1 → emoji localeCompare
    assert.deepEqual(
      next.map((g) => g.emoji),
      ["😂", "👍"].sort((a, b) => a.localeCompare(b)),
    );
  });
});

describe("reactionActorsLabel", () => {
  it("lists named actors when fully sampled", () => {
    assert.equal(
      reactionActorsLabel(
        group({
          emoji: "👍",
          count: 2,
          actors: [
            { id: "me", displayName: "You" },
            { id: "u1", displayName: "Alice" },
          ],
        }),
      ),
      "You and Alice",
    );
  });

  it("shows remaining count when sample is partial", () => {
    assert.equal(
      reactionActorsLabel(
        group({
          emoji: "👍",
          count: 5,
          actors: [{ id: "me", displayName: "You" }],
        }),
      ),
      "You and 4 others",
    );
  });
});

describe("mapDaemonReactionGroup", () => {
  it("maps local actor to ME_ACTOR", () => {
    const ui = mapDaemonReactionGroup({
      emoji: "👍",
      count: 1,
      reactedByMe: true,
      actors: [
        {
          actorId: "local",
          actorKind: "user",
          displayName: "You",
        },
      ],
    });
    assert.equal(ui.reactedByMe, true);
    assert.deepEqual(ui.actors, [ME_ACTOR]);
  });
});

describe("shouldApplyToggleResponse", () => {
  it("applies only when request gen is still current", () => {
    assert.equal(shouldApplyToggleResponse(3, 3), true);
    assert.equal(shouldApplyToggleResponse(2, 3), false);
    assert.equal(shouldApplyToggleResponse(4, 3), false);
  });
});

describe("hasInFlightToggleCount", () => {
  it("is true only for positive counts", () => {
    assert.equal(hasInFlightToggleCount(undefined), false);
    assert.equal(hasInFlightToggleCount(0), false);
    assert.equal(hasInFlightToggleCount(1), true);
    assert.equal(hasInFlightToggleCount(2), true);
  });
});

describe("hydrateReactionsFromMessages", () => {
  it("merges daemon snapshots and clears empty", () => {
    const prev: Record<string, ReactionGroup[]> = {
      m1: [group({ emoji: "👀", reactedByMe: false })],
      m2: [group({ emoji: "🎉", reactedByMe: true })],
    };
    const next = hydrateReactionsFromMessages(prev, [
      {
        id: "m1",
        reactions: [
          {
            emoji: "👍",
            count: 1,
            reactedByMe: true,
            actors: [
              { actorId: "local", actorKind: "user", displayName: "You" },
            ],
          },
        ],
      },
      { id: "m2", reactions: [] },
    ]);
    assert.equal(next.m1!.length, 1);
    assert.equal(next.m1![0]!.emoji, "👍");
    assert.equal(next.m1![0]!.actors[0]!.id, ME_ACTOR.id);
    assert.equal(next.m2, undefined);
  });

  it("skips messages with in-flight toggles", () => {
    const prev: Record<string, ReactionGroup[]> = {
      m1: [group({ emoji: "👍", reactedByMe: true })],
    };
    const next = hydrateReactionsFromMessages(
      prev,
      [
        {
          id: "m1",
          reactions: [],
        },
      ],
      { skipMessageIds: new Set(["m1"]) },
    );
    assert.equal(next.m1![0]!.emoji, "👍");
  });
});
