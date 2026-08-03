import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import { hubDigestCache } from "./hub-digest-cache.ts";
import { mergeConversationList } from "./conversation-list-merge.ts";

// Pure helpers only — no @/ path aliases (node:test strip-types).

describe("hubDigestCache", () => {
  beforeEach(() => {
    hubDigestCache._resetForTests();
  });

  it("hydrates once and patches live deltas", () => {
    assert.equal(hubDigestCache.isHydrated(), false);
    hubDigestCache.hydrate([
      {
        conversationId: "c1",
        title: "One",
        preview: "hi",
        lastMessageAtMs: 100,
        unreadCount: 1,
        unreadMentionCount: 0,
        kind: "group",
        memberCount: 2,
      },
    ]);
    assert.equal(hubDigestCache.isHydrated(), true);
    assert.equal(hubDigestCache.getAll().length, 1);

    hubDigestCache.patchOne("c1", {
      preview: "hello",
      lastMessageAtMs: 200,
      unreadCount: 2,
    });
    const row = hubDigestCache.get("c1");
    assert.equal(row?.preview, "hello");
    assert.equal(row?.unreadCount, 2);
    assert.equal(row?.title, "One");
  });

  it("invalidate clears hydrate flag", () => {
    hubDigestCache.hydrate([
      {
        conversationId: "c1",
        title: "One",
        preview: null,
        lastMessageAtMs: 1,
        unreadCount: 0,
        unreadMentionCount: 0,
        kind: "group",
        memberCount: 1,
      },
    ]);
    hubDigestCache.invalidate();
    assert.equal(hubDigestCache.isHydrated(), false);
    assert.equal(hubDigestCache.getAll().length, 0);
  });
});

describe("mergeConversationList", () => {
  it("prefers Hub title/preview/unread and keeps daemon host fields", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      daemonRows: [
        {
          id: "c1",
          projectId: "p1",
          title: "Daemon title",
          preview: "daemon preview",
          updatedAtMs: 50,
          participatingAgents: ["codex"],
          runningCount: 1,
          approvalCount: 0,
          messageCount: 3,
        },
      ],
      hubDigests: [
        {
          conversationId: "c1",
          title: "Hub title",
          preview: "hub preview",
          lastMessageAtMs: 100,
          unreadCount: 2,
          unreadMentionCount: 0,
          kind: "group",
          memberCount: 2,
        },
      ],
    });
    assert.equal(merged.length, 1);
    assert.equal(merged[0].title, "Hub title");
    assert.equal(merged[0].preview, "hub preview");
    assert.equal(merged[0].unread, 2);
    assert.deepEqual(merged[0].participatingAgents, ["codex"]);
    assert.equal(merged[0].runningCount, 1);
  });

  it("shows Hub-only rows when daemon absent", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      daemonRows: [],
      hubDigests: [
        {
          conversationId: "hub-only",
          title: "Mobile chat",
          preview: "from phone",
          lastMessageAtMs: 300,
          unreadCount: 1,
          unreadMentionCount: 0,
          kind: "group",
          memberCount: 1,
        },
      ],
      includeHubOnly: true,
    });
    assert.equal(merged.length, 1);
    assert.equal(merged[0].id, "hub-only");
    assert.equal(merged[0].title, "Mobile chat");
    assert.equal(merged[0].unread, 1);
    assert.equal(merged[0].participatingAgents.length, 0);
  });
});
