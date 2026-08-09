import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  agentResultSessionKey,
  mergeHubAndLocalTimeline,
  removeMessageFromTimeline,
  sessionIdFromAgentResultId,
  upsertHubMessageIntoTimeline,
} from "./hub-timeline.ts";
import type { TimelineMessage } from "./mock-data.ts";

function local(
  partial: Partial<TimelineMessage> & Pick<TimelineMessage, "id">,
): TimelineMessage {
  return {
    role: "user",
    body: partial.body ?? partial.id,
    time: "now",
    createdAtMs: partial.createdAtMs ?? 0,
    ...partial,
  };
}

describe("agentResultSessionKey", () => {
  it("parses conversation:session prefix", () => {
    assert.equal(
      agentResultSessionKey("agent-result:c:s:origin"),
      "c:s",
    );
    assert.equal(sessionIdFromAgentResultId("agent-result:c:s:origin"), "s");
  });
});

describe("mergeHubAndLocalTimeline", () => {
  it("prefers hub on same id; keeps local tool + agent-result gap by id", () => {
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({ id: "hub-u", role: "user", body: "from mobile", createdAtMs: 10 }),
        local({
          id: "agent-result:c:sess1:origin1",
          role: "agent",
          body: "hub reply",
          createdAtMs: 20,
        }),
      ],
      localMessages: [
        local({
          id: "tool-1",
          role: "system",
          kind: "tool_summary",
          body: "rg",
          createdAtMs: 15,
        }),
        local({
          id: "agent-result:c:sess1:origin1",
          role: "agent",
          body: "local duplicate",
          createdAtMs: 21,
        }),
        local({
          // Different origin (second turn) — keep until Hub has same id.
          id: "agent-result:c:sess1:origin2",
          role: "agent",
          body: "local only reply",
          createdAtMs: 22,
        }),
      ],
    });
    const ids = merged.map((m) => m.id);
    assert.ok(ids.includes("hub-u"));
    assert.ok(ids.includes("tool-1"));
    // Same id: Hub body wins.
    assert.equal(
      merged.find((m) => m.id === "agent-result:c:sess1:origin1")?.body,
      "hub reply",
    );
    // Different origin id: both kept (no session soft-dedupe).
    assert.ok(ids.includes("agent-result:c:sess1:origin2"));
  });

  it("Hub empty reactions win over stale local reactions", () => {
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({
          id: "m1",
          role: "user",
          body: "hi",
          createdAtMs: 1,
          reactions: [],
        }),
      ],
      localMessages: [
        local({
          id: "m1",
          role: "user",
          body: "hi",
          createdAtMs: 1,
          reactions: [
            {
              emoji: "👍",
              count: 1,
              reactedByMe: true,
              actors: [{ id: "me", displayName: "Me" }],
            },
          ],
        }),
      ],
    });
    const row = merged.find((m) => m.id === "m1");
    assert.ok(row);
    assert.deepEqual(row?.reactions, []);
  });

  it("uses Hub messageSeq as social SSOT (not host daemon seq)", () => {
    // Hub insert order is the multi-end social order. Host finish seq stays
    // local-only and must not overwrite Hub message_seq on same id.
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({
          id: "msg_user",
          role: "user",
          body: "@grok hi",
          createdAtMs: 100,
          messageSeq: 10,
        }),
        local({
          id: "agent-result:c:s:msg_user",
          role: "agent",
          body: "hub reply",
          createdAtMs: 200,
          messageSeq: 11,
        }),
      ],
      localMessages: [
        local({
          id: "msg_user",
          role: "user",
          body: "@grok hi",
          createdAtMs: 100,
          messageSeq: 8,
        }),
        local({
          id: "agent-result:c:s:msg_user",
          role: "agent",
          body: "local reply",
          createdAtMs: 200,
          messageSeq: 9,
        }),
      ],
    });
    const user = merged.find((m) => m.id === "msg_user");
    const agent = merged.find((m) => m.id === "agent-result:c:s:msg_user");
    assert.equal(user?.messageSeq, 10, "hub seq is social SSOT");
    assert.equal(agent?.messageSeq, 11, "hub seq is social SSOT");
    assert.equal(agent?.body, "hub reply", "hub body still wins on same id");
    assert.equal(merged[0]?.id, "msg_user");
    assert.equal(merged[1]?.id, "agent-result:c:s:msg_user");
  });

  it("keeps hub-only mobile messageSeq for social order", () => {
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({
          id: "mobile-only",
          role: "user",
          body: "from phone",
          createdAtMs: 150,
          messageSeq: 11,
        }),
        local({
          id: "host-user",
          role: "user",
          body: "@grok hi",
          createdAtMs: 100,
          messageSeq: 10,
        }),
      ],
      localMessages: [
        local({
          id: "host-user",
          role: "user",
          body: "@grok hi",
          createdAtMs: 100,
          messageSeq: 8,
        }),
        local({
          id: "agent-result:c:s:host-user",
          role: "agent",
          body: "reply",
          createdAtMs: 200,
          messageSeq: 9,
        }),
      ],
    });
    const mobile = merged.find((m) => m.id === "mobile-only");
    assert.equal(
      mobile?.messageSeq,
      11,
      "hub-only peer keeps Hub message_seq",
    );
    const ids = merged.map((m) => m.id);
    assert.deepEqual(ids, [
      "host-user",
      "mobile-only",
      "agent-result:c:s:host-user",
    ]);
  });

  it("drops bare non-canonical agent locals not on hub (no dual SSOT ghosts)", () => {
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({ id: "hub-u", role: "user", body: "hi", createdAtMs: 1 }),
      ],
      localMessages: [
        local({
          id: "orphan-agent-uuid",
          role: "agent",
          body: "ghost",
          createdAtMs: 2,
        }),
        local({
          id: "agent-result:c:s:origin",
          role: "agent",
          body: "keep until hub",
          createdAtMs: 3,
        }),
      ],
    });
    const ids = merged.map((m) => m.id);
    assert.ok(ids.includes("hub-u"));
    assert.ok(ids.includes("agent-result:c:s:origin"));
    assert.equal(ids.includes("orphan-agent-uuid"), false);
  });

  it("does not soft-dedupe by body or session when origin ids differ", () => {
    // Pre-C2: same session different durable suffix suppressed local.
    // C2: only same id collapses.
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({
          id: "agent-result:conv1:sessA:user-msg-1",
          role: "agent",
          body: "Hi again — ready when you are.",
          createdAtMs: 100,
          replyToMessageId: "user-msg-1",
        }),
      ],
      localMessages: [
        local({
          id: "agent-result:conv1:sessA:user-msg-2",
          role: "agent",
          body: "Hi again — ready when you are.",
          createdAtMs: 101,
        }),
      ],
    });
    const agentRows = merged.filter((m) => m.role === "agent");
    assert.equal(agentRows.length, 2);
  });

  it("keeps optimistic sending/failed user rows not yet on hub", () => {
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [],
      localMessages: [
        local({
          id: "pending-1",
          role: "user",
          body: "…",
          deliveryStatus: "sending",
          createdAtMs: 1,
        }),
      ],
    });
    assert.equal(merged.length, 1);
    assert.equal(merged[0]?.id, "pending-1");
  });

  it("hub and local shared id: hub wins; local-only user gap-fills", () => {
    const hub = local({ id: "shared-1", role: "user", body: "from hub", createdAtMs: 100 });
    const localDup = local({
      id: "shared-1",
      role: "user",
      body: "from local",
      createdAtMs: 100,
    });
    const localOnlyUser = local({
      id: "local-only-user",
      role: "user",
      body: "local",
      createdAtMs: 101,
    });
    const tool = local({
      id: "tool-1",
      role: "system",
      kind: "tool_summary",
      body: "rg",
      createdAtMs: 102,
    });
    const optimistic = local({
      id: "pending-1",
      role: "user",
      body: "…",
      deliveryStatus: "sending",
      createdAtMs: 202,
    });
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [hub],
      localMessages: [localDup, localOnlyUser, tool, optimistic],
    });
    const ids = merged.map((m) => m.id);
    assert.ok(ids.includes("shared-1"));
    assert.equal(merged.find((m) => m.id === "shared-1")?.body, "from hub");
    assert.ok(ids.includes("tool-1"));
    assert.ok(ids.includes("pending-1"));
    assert.ok(ids.includes("local-only-user"));
  });
});

describe("upsertHubMessageIntoTimeline", () => {
  it("inserts and updates by id", () => {
    const a = upsertHubMessageIntoTimeline([], local({ id: "m1", body: "a", createdAtMs: 1 }));
    assert.equal(a.length, 1);
    const b = upsertHubMessageIntoTimeline(a, local({ id: "m1", body: "b", createdAtMs: 1 }));
    assert.equal(b.length, 1);
    assert.equal(b[0]?.body, "b");
  });

  it("keeps distinct agent-result origins (no session soft drop)", () => {
    const prev = [
      local({
        id: "agent-result:c:s1:origin-local",
        role: "agent",
        body: "done",
        createdAtMs: 10,
      }),
    ];
    const next = upsertHubMessageIntoTimeline(
      prev,
      local({
        id: "agent-result:c:s1:origin-hub",
        role: "agent",
        body: "done",
        createdAtMs: 11,
        replyToMessageId: "u1",
      }),
    );
    assert.equal(next.length, 2);
    assert.ok(next.some((m) => m.id === "agent-result:c:s1:origin-local"));
    assert.ok(next.some((m) => m.id === "agent-result:c:s1:origin-hub"));
  });
});

describe("removeMessageFromTimeline", () => {
  it("removes by id for recall", () => {
    const list = [
      local({ id: "m1", body: "a" }),
      local({ id: "m2", body: "b" }),
    ];
    const next = removeMessageFromTimeline(list, "m1");
    assert.equal(next.length, 1);
    assert.equal(next[0]?.id, "m2");
    assert.equal(removeMessageFromTimeline(list, "missing"), list);
  });
});
