import assert from "node:assert/strict";
import { describe, it, beforeEach } from "node:test";
import { cloudDigestCache } from "./cloud-digest-cache.ts";
import {
  mergeConversationList,
  resolveLastActivityMs,
  resolveListPreview,
  resolveRailUnread,
} from "./conversation-list-merge.ts";

// Pure helpers only — no @/ path aliases (node:test strip-types).

describe("cloudDigestCache", () => {
  beforeEach(() => {
    cloudDigestCache._resetForTests();
  });

  it("hydrates once and patches live deltas", () => {
    assert.equal(cloudDigestCache.isHydrated(), false);
    cloudDigestCache.hydrate([
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
    assert.equal(cloudDigestCache.isHydrated(), true);
    assert.equal(cloudDigestCache.getAll().length, 1);

    cloudDigestCache.patchOne("c1", {
      preview: "hello",
      lastMessageAtMs: 200,
      unreadCount: 2,
    });
    const row = cloudDigestCache.get("c1");
    assert.equal(row?.preview, "hello");
    assert.equal(row?.unreadCount, 2);
    assert.equal(row?.title, "One");
  });

  it("isHydratedFor requires matching owner account", () => {
    cloudDigestCache.hydrate(
      [
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
      ],
      "acct-a",
    );
    assert.equal(cloudDigestCache.isHydratedFor("acct-a"), true);
    assert.equal(cloudDigestCache.isHydratedFor("acct-b"), false);
    cloudDigestCache.invalidate();
    assert.equal(cloudDigestCache.isHydratedFor("acct-a"), false);
  });

  it("invalidate clears hydrate flag", () => {
    cloudDigestCache.hydrate([
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
    cloudDigestCache.invalidate();
    assert.equal(cloudDigestCache.isHydrated(), false);
    assert.equal(cloudDigestCache.getAll().length, 0);
  });
});

describe("resolveRailUnread (single-track)", () => {
  it("hub mode uses digest only and ignores local baseline", () => {
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        unreadSource: "hub",
        cloudUnreadCount: 3,
        localUnread: 99,
      }),
      3,
    );
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        unreadSource: "hub",
        cloudUnreadCount: 0,
        localUnread: 99,
      }),
      undefined,
    );
    assert.equal(
      resolveRailUnread({
        conversationId: "c1",
        unreadSource: "hub",
        cloudUnreadCount: undefined,
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
        cloudUnreadCount: 7,
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
        cloudUnreadCount: 4,
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
        cloud: {
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
        cloudLastMessageAtMs: 50,
        daemonUpdatedAtMs: 200,
      }),
      "local preview",
    );
  });

  it("uses hub preview when hub is newer or tied", () => {
    assert.equal(
      resolveListPreview({
        cloud: {
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
        cloudLastMessageAtMs: 200,
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
          participatingBots: [
            { botId: "local-rt-codex", name: "codex", runtime: "codex" },
          ],
          participatingAgents: ["codex"],
          runningCount: 1,
          approvalCount: 0,
          messageCount: 3,
          unread: 9, // local baseline must not win in hub mode
        },
      ],
      cloudDigests: [
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

  it("keeps daemon title when Hub only has Conversation placeholder", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "hub",
      daemonRows: [
        {
          id: "c1",
          projectId: "p1",
          title: "JWT auth refactor",
          preview: "local",
          updatedAtMs: 50,
        },
      ],
      cloudDigests: [
        {
          conversationId: "c1",
          title: "Conversation",
          preview: "hub",
          lastMessageAtMs: 100,
          unreadCount: 0,
          unreadMentionCount: 0,
          kind: "group",
          memberCount: 1,
        },
      ],
    });
    assert.equal(merged[0].title, "JWT auth refactor");
  });

  it("keeps daemon title when Hub title is empty", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "hub",
      daemonRows: [
        {
          id: "c1",
          projectId: "p1",
          title: "Local named chat",
        },
      ],
      cloudDigests: [
        {
          conversationId: "c1",
          title: "",
          preview: null,
          lastMessageAtMs: 0,
          unreadCount: 0,
          unreadMentionCount: 0,
          kind: "group",
          memberCount: 1,
        },
      ],
    });
    assert.equal(merged[0].title, "Local named chat");
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
      cloudDigests: [
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
      cloudDigests: [],
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
      cloudDigests: [],
    });
    assert.equal(merged[0].unread, 4);
  });

  it("shows Hub-only rows when daemon absent", () => {
    const merged = mergeConversationList({
      projectId: "p1",
      unreadSource: "hub",
      daemonRows: [],
      cloudDigests: [
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
      includeCloudOnly: true,
    });
    assert.equal(merged.length, 1);
    assert.equal(merged[0].id, "hub-only");
    assert.equal(merged[0].title, "Mobile chat");
    assert.equal(merged[0].unread, 1);
    assert.equal(merged[0].participatingAgents.length, 0);
  });
});
