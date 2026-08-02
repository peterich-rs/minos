import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  hubChatMessageToTimeline,
  isHubImMode,
  isLocalChatBubbleForHubSsot,
  mergeHubAndLocalTimeline,
  removeMessageFromTimeline,
  upsertHubMessageIntoTimeline,
} from "./hub-timeline.ts";
import type { TimelineMessage } from "./mock-data.ts";
import type { HubChatMessage } from "./minos-cloud.ts";

function hub(partial: Partial<HubChatMessage> & Pick<HubChatMessage, "messageId">): HubChatMessage {
  return {
    messageId: partial.messageId,
    conversationId: partial.conversationId ?? "c1",
    text: partial.text ?? "hello",
    createdAtMs: partial.createdAtMs ?? 1_000,
    senderType: partial.senderType ?? "user",
    senderAccountId: partial.senderAccountId ?? "acc",
    senderMinosId: partial.senderMinosId ?? "u",
    senderDisplayName: partial.senderDisplayName ?? "User",
    replyToMessageId: partial.replyToMessageId ?? null,
    recalledAtMs: partial.recalledAtMs ?? null,
  };
}

function local(
  partial: Partial<TimelineMessage> & Pick<TimelineMessage, "id">,
): TimelineMessage {
  return {
    id: partial.id,
    role: partial.role ?? "user",
    body: partial.body ?? "x",
    time: partial.time ?? "now",
    createdAtMs: partial.createdAtMs ?? 1,
    kind: partial.kind,
    agent: partial.agent,
    sessionId: partial.sessionId,
    deliveryStatus: partial.deliveryStatus,
    replyToMessageId: partial.replyToMessageId,
    messageSeq: partial.messageSeq,
  };
}

describe("isHubImMode", () => {
  it("requires authenticated + access token", () => {
    assert.equal(isHubImMode({ authPhase: "authenticated", accessToken: "t" }), true);
    assert.equal(isHubImMode({ authPhase: "login", accessToken: "t" }), false);
    assert.equal(isHubImMode({ authPhase: "authenticated", accessToken: "" }), false);
  });
});

describe("hubChatMessageToTimeline", () => {
  it("maps user and agent rows", () => {
    const u = hubChatMessageToTimeline(hub({ messageId: "m1", text: "hi" }));
    assert.equal(u?.role, "user");
    assert.equal(u?.body, "hi");
    assert.equal(u?.kind, "text");

    const a = hubChatMessageToTimeline(
      hub({
        messageId: "m2",
        text: "done",
        senderType: "agent",
        senderDisplayName: "🤖 Grok",
      }),
    );
    assert.equal(a?.role, "agent");
    assert.equal(a?.agent, "grok");
  });

  it("skips empty and recalled", () => {
    assert.equal(
      hubChatMessageToTimeline(hub({ messageId: "m", text: "  " })),
      null,
    );
    assert.equal(
      hubChatMessageToTimeline(
        hub({ messageId: "m", text: "x", recalledAtMs: 99 }),
      ),
      null,
    );
  });
});

describe("isLocalChatBubbleForHubSsot", () => {
  it("treats user/agent text as hub-owned bubbles", () => {
    assert.equal(
      isLocalChatBubbleForHubSsot(local({ id: "u1", role: "user" })),
      true,
    );
    assert.equal(
      isLocalChatBubbleForHubSsot(
        local({ id: "agent-result:c:s:t", role: "agent" }),
      ),
      true,
    );
  });

  it("keeps tool/git/approval local", () => {
    assert.equal(
      isLocalChatBubbleForHubSsot(
        local({ id: "t1", role: "system", kind: "tool_summary" }),
      ),
      false,
    );
    assert.equal(
      isLocalChatBubbleForHubSsot(
        local({ id: "g1", role: "system", kind: "git_activity" }),
      ),
      false,
    );
  });
});

describe("mergeHubAndLocalTimeline rebuild", () => {
  it("hub wins same id; gap-fills local chat missing from hub", () => {
    const hub: TimelineMessage = {
      id: "shared-1",
      role: "user",
      body: "from hub",
      time: "12:00",
      createdAtMs: 200,
      kind: "text",
      deliveryStatus: "sent",
    };
    const localDup: TimelineMessage = {
      id: "shared-1",
      role: "user",
      body: "local older copy",
      time: "11:00",
      createdAtMs: 100,
      kind: "text",
      deliveryStatus: "sent",
    };
    const localOnlyUser: TimelineMessage = {
      id: "local-only-user",
      role: "user",
      body: "not on hub yet",
      time: "11:00",
      createdAtMs: 100,
      kind: "text",
      deliveryStatus: "sent",
    };
    const tool: TimelineMessage = {
      id: "tool-1",
      role: "system",
      body: "ran tool",
      time: "12:01",
      createdAtMs: 201,
      kind: "tool_summary",
    };
    const optimistic: TimelineMessage = {
      id: "pending-1",
      role: "user",
      body: "sending…",
      time: "12:02",
      createdAtMs: 202,
      kind: "text",
      deliveryStatus: "sending",
    };
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [hub],
      localMessages: [localDup, localOnlyUser, tool, optimistic],
    });
    const ids = merged.map((m) => m.id);
    assert.ok(ids.includes("shared-1"));
    assert.equal(merged.find((m) => m.id === "shared-1")?.body, "from hub");
    assert.ok(ids.includes("tool-1"));
    assert.ok(ids.includes("pending-1"));
    // Gap-fill: local user not on hub still shows (Desktop native path).
    assert.ok(ids.includes("local-only-user"));
  });
});

describe("mergeHubAndLocalTimeline", () => {
  it("prefers hub on same id; keeps local tool + agent-result gap", () => {
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({ id: "hub-u", role: "user", body: "from mobile", createdAtMs: 10 }),
        local({
          id: "agent-result:c:sess1:1",
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
          id: "agent-result:c:sess1:1",
          role: "agent",
          body: "local duplicate",
          createdAtMs: 21,
        }),
        local({
          // Different session — Desktop-native turn not yet on Hub.
          id: "agent-result:c:sess2:1",
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
      merged.find((m) => m.id === "agent-result:c:sess1:1")?.body,
      "hub reply",
    );
    assert.ok(ids.includes("agent-result:c:sess2:1"));
  });

  it("suppresses local agent-result when Hub has same session different durable id", () => {
    // Mobile client_live path: Hub projector uses trigger_seq durable suffix;
    // daemon conversation_completion uses message_key — same turn, two ids.
    const merged = mergeHubAndLocalTimeline({
      hubMessages: [
        local({
          id: "agent-result:conv1:sessA:42",
          role: "agent",
          body: "Hi again — ready when you are.",
          createdAtMs: 100,
          replyToMessageId: "user-msg-1",
        }),
      ],
      localMessages: [
        local({
          id: "agent-result:conv1:sessA:m2",
          role: "agent",
          body: "Hi again — ready when you are.",
          createdAtMs: 101,
        }),
      ],
    });
    const agentRows = merged.filter((m) => m.role === "agent");
    assert.equal(agentRows.length, 1);
    assert.equal(agentRows[0]?.id, "agent-result:conv1:sessA:42");
    assert.equal(agentRows[0]?.replyToMessageId, "user-msg-1");
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
});

describe("upsertHubMessageIntoTimeline", () => {
  it("inserts and updates by id", () => {
    const a = upsertHubMessageIntoTimeline([], local({ id: "m1", body: "a", createdAtMs: 1 }));
    assert.equal(a.length, 1);
    const b = upsertHubMessageIntoTimeline(a, local({ id: "m1", body: "b", createdAtMs: 1 }));
    assert.equal(b.length, 1);
    assert.equal(b[0]?.body, "b");
  });

  it("drops local agent-result sibling when Hub agent-result for same session arrives", () => {
    const prev = [
      local({
        id: "agent-result:c:s1:localKey",
        role: "agent",
        body: "done",
        createdAtMs: 10,
      }),
    ];
    const next = upsertHubMessageIntoTimeline(
      prev,
      local({
        id: "agent-result:c:s1:99",
        role: "agent",
        body: "done",
        createdAtMs: 11,
        replyToMessageId: "u1",
      }),
    );
    assert.equal(next.length, 1);
    assert.equal(next[0]?.id, "agent-result:c:s1:99");
    assert.equal(next[0]?.replyToMessageId, "u1");
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
