import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TimelineMessage } from "../../../shared/lib/mock-data.ts";
import {
  formatDayDividerLabel,
  isMessageGroupContinuation,
  localDayKey,
  MESSAGE_GROUP_WINDOW_MS,
  messageAuthorKey,
  shouldShowDayDivider,
} from "./message-grouping.ts";

function msg(
  partial: Partial<TimelineMessage> & Pick<TimelineMessage, "id">,
): TimelineMessage {
  return {
    role: "user",
    body: partial.body ?? partial.id,
    time: "now",
    ...partial,
  };
}

describe("messageAuthorKey", () => {
  it("groups all user messages together", () => {
    assert.equal(messageAuthorKey(msg({ id: "a", role: "user" })), "user");
  });

  it("separates agents by session", () => {
    assert.equal(
      messageAuthorKey(
        msg({ id: "a", role: "agent", agent: "codex", sessionId: "s1" }),
      ),
      "agent:codex:s1",
    );
    assert.notEqual(
      messageAuthorKey(
        msg({ id: "a", role: "agent", agent: "codex", sessionId: "s1" }),
      ),
      messageAuthorKey(
        msg({ id: "b", role: "agent", agent: "codex", sessionId: "s2" }),
      ),
    );
  });

  it("prefers global bot_id over runtime family", () => {
    assert.equal(
      messageAuthorKey(
        msg({
          id: "a",
          role: "agent",
          agent: "codex",
          botId: "bot-a",
          sessionId: "s1",
        }),
      ),
      "agent:bot-a:s1",
    );
    assert.notEqual(
      messageAuthorKey(
        msg({
          id: "a",
          role: "agent",
          agent: "codex",
          botId: "bot-a",
          sessionId: "s1",
        }),
      ),
      messageAuthorKey(
        msg({
          id: "b",
          role: "agent",
          agent: "codex",
          botId: "bot-b",
          sessionId: "s1",
        }),
      ),
    );
  });

  it("returns null for system and tool_summary", () => {
    assert.equal(
      messageAuthorKey(msg({ id: "s", role: "system" })),
      null,
    );
    assert.equal(
      messageAuthorKey(
        msg({ id: "t", role: "agent", kind: "tool_summary" }),
      ),
      null,
    );
  });
});

describe("isMessageGroupContinuation", () => {
  const t0 = 1_700_000_000_000;

  it("is false without prev", () => {
    assert.equal(
      isMessageGroupContinuation(undefined, msg({ id: "a" })),
      false,
    );
  });

  it("collapses consecutive same-author within window", () => {
    const prev = msg({ id: "a", role: "user", createdAtMs: t0 });
    const curr = msg({
      id: "b",
      role: "user",
      createdAtMs: t0 + MESSAGE_GROUP_WINDOW_MS - 1,
    });
    assert.equal(isMessageGroupContinuation(prev, curr), true);
  });

  it("breaks after the time window", () => {
    const prev = msg({ id: "a", role: "user", createdAtMs: t0 });
    const curr = msg({
      id: "b",
      role: "user",
      createdAtMs: t0 + MESSAGE_GROUP_WINDOW_MS + 1,
    });
    assert.equal(isMessageGroupContinuation(prev, curr), false);
  });

  it("does not group different authors", () => {
    const prev = msg({ id: "a", role: "user", createdAtMs: t0 });
    const curr = msg({
      id: "b",
      role: "agent",
      agent: "codex",
      sessionId: "s1",
      createdAtMs: t0 + 1000,
    });
    assert.equal(isMessageGroupContinuation(prev, curr), false);
  });

  it("groups without timestamps when author matches", () => {
    const prev = msg({ id: "a", role: "user" });
    const curr = msg({ id: "b", role: "user" });
    assert.equal(isMessageGroupContinuation(prev, curr), true);
  });
});

describe("day dividers", () => {
  it("localDayKey formats local calendar day", () => {
    const d = new Date(2024, 5, 3, 15, 0, 0); // June 3 local
    assert.equal(localDayKey(d.getTime()), "2024-06-03");
    assert.equal(localDayKey(undefined), null);
    assert.equal(localDayKey(0), null);
  });

  it("shouldShowDayDivider on first message with timestamp", () => {
    const curr = msg({ id: "a", createdAtMs: Date.now() });
    assert.equal(shouldShowDayDivider(undefined, curr), true);
  });

  it("shouldShowDayDivider false same day", () => {
    const t = new Date(2024, 0, 10, 9, 0).getTime();
    const prev = msg({ id: "a", createdAtMs: t });
    const curr = msg({
      id: "b",
      createdAtMs: new Date(2024, 0, 10, 18, 0).getTime(),
    });
    assert.equal(shouldShowDayDivider(prev, curr), false);
  });

  it("shouldShowDayDivider true across days", () => {
    const prev = msg({
      id: "a",
      createdAtMs: new Date(2024, 0, 10, 23, 0).getTime(),
    });
    const curr = msg({
      id: "b",
      createdAtMs: new Date(2024, 0, 11, 1, 0).getTime(),
    });
    assert.equal(shouldShowDayDivider(prev, curr), true);
  });

  it("formatDayDividerLabel returns Today for now", () => {
    assert.equal(formatDayDividerLabel(Date.now()), "Today");
  });
});
