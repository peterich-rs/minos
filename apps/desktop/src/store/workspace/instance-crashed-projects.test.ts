/**
 * Pure coverage for the instanceCrashed projects rollup invariant.
 * Mirrors recomputeConversationAggregates + patchProjectAggregates policy
 * without loading projection.ts (@/ imports are unavailable under node:test).
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  conversationAggregatesFromEntities,
  entityCountsAsRunning,
  type SessionEntity,
} from "../../shared/lib/session-entity.ts";

function entity(
  partial: Partial<SessionEntity> & {
    sessionId: string;
    daemonStatus: SessionEntity["daemonStatus"];
  },
): SessionEntity {
  const daemonStatus = partial.daemonStatus;
  return {
    sessionId: partial.sessionId,
    conversationId: partial.conversationId ?? "c1",
    agent: partial.agent ?? "codex",
    shortId: partial.shortId ?? "abcd1234",
    status: daemonStatus,
    daemonStatus,
    model: partial.model ?? "m",
    summary: partial.summary ?? "",
    messageCount: partial.messageCount ?? 0,
    hasPendingApproval: partial.hasPendingApproval ?? false,
    updatedAtMs: partial.updatedAtMs ?? 1,
  };
}

describe("instanceCrashed project rollup invariant", () => {
  it("suspended sessions stop counting as running for project badges", () => {
    const running = entity({ sessionId: "s1", daemonStatus: "running" });
    assert.equal(entityCountsAsRunning(running), true);
    const { runningCount } = conversationAggregatesFromEntities(
      { s1: running },
      "c1",
    );
    assert.equal(runningCount, 1);

    const suspended = entity({ sessionId: "s1", daemonStatus: "suspended" });
    assert.equal(entityCountsAsRunning(suspended), false);
    const after = conversationAggregatesFromEntities({ s1: suspended }, "c1");
    assert.equal(after.runningCount, 0);
  });
});
