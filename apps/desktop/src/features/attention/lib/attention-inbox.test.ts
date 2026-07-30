import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildAttentionInbox,
  conversationAttentionScore,
  countAttentionInboxByCategory,
  filterAttentionInbox,
} from "./attention-inbox.ts";

describe("buildAttentionInbox", () => {
  const projects = [
    { id: "p1", name: "Minos" },
    { id: "p2", name: "Other" },
  ];
  const conversations = [
    {
      id: "c1",
      projectId: "p1",
      title: "Auth",
      preview: "codex finished routes",
      updatedAtMs: 100,
      unread: 2,
      approvalCount: 1,
    },
    {
      id: "c2",
      projectId: "p1",
      title: "Docs",
      preview: "draft ready",
      updatedAtMs: 50,
      unread: 0,
    },
    {
      id: "c3",
      projectId: "p2",
      title: "Quiet",
      preview: "hi",
      updatedAtMs: 200,
      unread: 5,
    },
  ];
  const sessions = [
    {
      id: "s1",
      conversationId: "c1",
      agent: "codex",
      shortId: "abc",
      status: "needs_approval",
      summary: "Write package.json?",
      lastTsMs: 150,
    },
    {
      id: "s2",
      conversationId: "c2",
      agent: "claude",
      shortId: "def",
      status: "failed",
      summary: "tool error",
      lastTsMs: 40,
    },
    {
      id: "s3",
      conversationId: "c2",
      agent: "gemini",
      shortId: "ghi",
      status: "running",
      summary: "should ignore",
      lastTsMs: 999,
    },
  ];

  it("emits session + unread rows", () => {
    const items = buildAttentionInbox({ conversations, projects, sessions });
    const ids = items.map((i) => i.id);
    assert.ok(ids.includes("session:s1"));
    assert.ok(ids.includes("session:s2"));
    assert.ok(!ids.includes("session:s3"));
    assert.ok(ids.includes("unread:c1"));
    assert.ok(ids.includes("unread:c3"));
    assert.ok(!ids.includes("unread:c2"));
  });

  it("sorts by updatedAtMs desc", () => {
    const items = buildAttentionInbox({ conversations, projects, sessions });
    for (let i = 1; i < items.length; i++) {
      assert.ok(items[i - 1]!.updatedAtMs >= items[i]!.updatedAtMs);
    }
  });

  it("labels project names", () => {
    const items = buildAttentionInbox({ conversations, projects, sessions });
    const unreadOther = items.find((i) => i.id === "unread:c3");
    assert.equal(unreadOther?.projectName, "Other");
    assert.equal(unreadOther?.title, "5 unread messages");
  });
});

describe("filterAttentionInbox / counts", () => {
  it("filters and counts", () => {
    const items = buildAttentionInbox({
      projects: [{ id: "p1", name: "P" }],
      conversations: [
        {
          id: "c1",
          projectId: "p1",
          title: "T",
          preview: "x",
          updatedAtMs: 1,
          unread: 1,
        },
      ],
      sessions: [
        {
          id: "s1",
          conversationId: "c1",
          agent: "codex",
          shortId: "a",
          status: "needs_approval",
          summary: "ok",
          lastTsMs: 2,
        },
      ],
    });
    assert.equal(filterAttentionInbox(items, "unread").length, 1);
    assert.equal(filterAttentionInbox(items, "approval").length, 1);
    assert.equal(filterAttentionInbox(items, "all").length, 2);
    const counts = countAttentionInboxByCategory(items);
    assert.equal(counts.all, 2);
    assert.equal(counts.approval, 1);
    assert.equal(counts.unread, 1);
  });
});

describe("conversationAttentionScore", () => {
  it("sums unread and approvals", () => {
    assert.equal(conversationAttentionScore({ unread: 2, approvalCount: 1 }), 3);
    assert.equal(conversationAttentionScore({}), 0);
  });
});
