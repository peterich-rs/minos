import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import { hubDigestCache } from "./hub-digest-cache.ts";
import {
  mergeConversationList,
  resolveLastActivityMs,
  resolveListPreview,
  resolveRailUnread,
} from "./conversation-list-merge.ts";

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

describe("resolveRailUnread (P1 single-track)", () => {
  it("hub mode uses digest only and ignores local baseline", () => {
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        unreadSource: "hub",
        hubUnreadCount: 3,
        localUnread: 99,
      }),
      3,
    );
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        unreadSource: "hub",
        hubUnreadCount: 0,
        localUnread: 99,
      }),
      undefined,
    );
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        unreadSource: "hub",
        hubUnreadCount: undefined,
        localUnread: 5,
      }),
      undefined,
    );
  });

  it("local mode uses baseline unread only", () => {
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        unreadSource: "local",
        hubUnreadCount: 7,
        localUnread: 2,
      }),
      2,
    );
  });

  it("focused conversation clears badge", () => {
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        focusedConversationId: "c1",
        unreadSource: "hub",
        hubUnreadCount: 4,
      }),
      undefined,
    );
  });
});

describe("resolveLastActivityMs / resolveListPreview", () => {
  it("takes max of hub and daemon for last activity", () => {
    assert.equal(resolveLastActivityMs(100, 50), 100);
    assert.equal(resolveLastActivityMs(50, 200), 200);
    assert.equal(resolveLastActivityMs(0, 0), 0);
    assert.equal(resolveLastActivityMs(undefined, 40), 40);
  });

  it("uses daemon preview when local activity is newer", () => {
    assert.equal(
      resolveListPreview({
        hub: {
          conversationId: "c1",
          title: "Hub",
          preview: "hub preview",
          lastMessageAtMs: 50,
          unreadCount: 0,
          unreadMentionCount: 0,
          kind: "group",
          memberCount: 1,
        },
        daemonPreview: "local preview",
        hubLastMessageAtMs: 50,
        daemonUpdatedAtMs: 200,
      }),
      "local preview",
    );
  });

  it("uses hub preview when hub is newer or tied", () => {
    assert.equal(
      resolveListPreview({
        hub: {
          conversationId: "c1",
          title: "Hub",
          preview: "hub preview",
          lastMessageAtMs: 200,
          unreadCount: 0,
          unreadMentionCount: 0,
          kind: "group",
          memberCount: 1,
        },
        daemonPreview: "local preview",
        hubLastMessageAtMs: 200,
        daemonUpdatedAtMs: 100,
      }),
      "hub preview",
    );
  });
});

describe("mergeConversationList", () => {
  it("prefers Hub title/unread; last activity is max(hub, daemon)", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "hub",
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
          unread: 9, // local baseline must not win in hub mode
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
    assert.equal(merged[0].updatedAtMs, 100);
    assert.equal(merged[0].unread, 2);
    assert.deepEqual(merged[0].participatingAgents, ["codex"]);
    assert.equal(merged[0].runningCount, 1);
  });

  it("does not pin list time to a stale Hub digest when daemon is newer", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "hub",
      daemonRows: [
        {
          id: "c1",
          projectId: "p1",
          title: "Local",
          preview: "just sent",
          updatedAtMs: 5000,
        },
      ],
      hubDigests: [
        {
          conversationId: "c1",
          title: "Hub title",
          preview: "old hub",
          lastMessageAtMs: 100,
          unreadCount: 0,
          unreadMentionCount: 0,
          kind: "group",
          memberCount: 1,
        },
      ],
    });
    assert.equal(merged[0].updatedAtMs, 5000);
    assert.equal(merged[0].preview, "just sent");
    assert.equal(merged[0].title, "Hub title");
  });

  it("hub mode does not fall back to local unread when digest missing", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "hub",
      daemonRows: [
        {
          id: "c-local-only",
          projectId: "p1",
          title: "Daemon",
          unread: 5,
          messageCount: 10,
        },
      ],
      hubDigests: [],
    });
    assert.equal(merged.length, 1);
    assert.equal(merged[0].unread, undefined);
  });

  it("local mode keeps daemon baseline unread", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "local",
      daemonRows: [
        {
          id: "c1",
          projectId: "p1",
          title: "Daemon",
          unread: 4,
        },
      ],
      hubDigests: [],
    });
    assert.equal(merged[0].unread, 4);
  });

  it("shows Hub-only rows when daemon absent", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "hub",
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
