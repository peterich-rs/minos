/**
 * Unit coverage for reset helpers that have no `@/` graph.
 * Full `resetWorkspaceModuleState` is wired in connection bootstrap (Tauri path);
 * path-alias modules are not loadable under plain node:test without a loader.
 */
import assert from "node:assert/strict";
import { describe, it, beforeEach, afterEach } from "node:test";
import {
  clearConversationRefreshTimers,
  conversationRefreshTimers,
} from "./empty-workspace.ts";
import {
  clearDesktopInflightState,
  resumeInFlightSessions,
  resumedInterruptedSessions,
  singleFlightLoad,
} from "../../shared/lib/desktop-inflight.ts";

describe("clearConversationRefreshTimers", () => {
  afterEach(() => {
    clearConversationRefreshTimers();
  });

  it("clears pending timeouts and empties the map", () => {
    let fired = 0;
    const handle = setTimeout(() => {
      fired += 1;
    }, 60_000);
    conversationRefreshTimers.set("conv-a", handle);
    assert.equal(conversationRefreshTimers.size, 1);
    clearConversationRefreshTimers();
    assert.equal(conversationRefreshTimers.size, 0);
    assert.equal(fired, 0);
  });
});

describe("clearDesktopInflightState", () => {
  beforeEach(() => {
    clearDesktopInflightState();
  });

  it("clears resume sets and load single-flight", async () => {
    resumedInterruptedSessions.add("s1");
    resumeInFlightSessions.add("s2");
    let ran = 0;
    const p = singleFlightLoad("k", async () => {
      ran += 1;
    });
    clearDesktopInflightState();
    assert.equal(resumedInterruptedSessions.size, 0);
    assert.equal(resumeInFlightSessions.size, 0);
    await p;
    assert.equal(ran, 1);
    let ran2 = 0;
    await singleFlightLoad("k", async () => {
      ran2 += 1;
    });
    assert.equal(ran2, 1);
  });
});
