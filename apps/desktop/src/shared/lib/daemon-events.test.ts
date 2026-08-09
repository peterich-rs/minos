/**
 * Event-bridge start atomicity: `started` only flips after every listen
 * succeeds; partial failure cleans up and leaves the bridge restartable.
 *
 * node:test cannot resolve @tauri-apps or @/ — exercise the pure control
 * logic with a local twin of startDaemonEventBridge's arm accumulation.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";

type UnlistenFn = () => void;

/**
 * Mirror of startDaemonEventBridge listen accumulation (daemon-events.ts).
 * Keep in sync: only set started after all arms attach; on failure unsub all.
 */
async function startBridgeAtomic(
  listens: Array<() => Promise<UnlistenFn>>,
  state: { started: boolean; unsubs: UnlistenFn[] },
): Promise<void> {
  if (state.started) return;
  const pending: UnlistenFn[] = [];
  try {
    for (const listen of listens) {
      pending.push(await listen());
    }
    state.unsubs = pending;
    state.started = true;
  } catch (err) {
    for (const u of pending) {
      try {
        u();
      } catch {
        /* ignore */
      }
    }
    state.unsubs = [];
    state.started = false;
    throw err;
  }
}

function stopBridge(state: { started: boolean; unsubs: UnlistenFn[] }): void {
  for (const u of state.unsubs) {
    try {
      u();
    } catch {
      /* ignore */
    }
  }
  state.unsubs = [];
  state.started = false;
}

describe("startDaemonEventBridge atomic start policy", () => {
  it("sets started only after all listens succeed", async () => {
    const state = { started: false, unsubs: [] as UnlistenFn[] };
    const cleaned: string[] = [];
    await startBridgeAtomic(
      [
        async () => () => cleaned.push("u1"),
        async () => () => cleaned.push("u2"),
        async () => () => cleaned.push("u3"),
        async () => () => cleaned.push("u4"),
      ],
      state,
    );
    assert.equal(state.started, true);
    assert.equal(state.unsubs.length, 4);
    assert.deepEqual(cleaned, []);
    stopBridge(state);
    assert.equal(state.started, false);
    assert.deepEqual(cleaned, ["u1", "u2", "u3", "u4"]);
  });

  it("on partial failure cleans up and leaves started false", async () => {
    const state = { started: false, unsubs: [] as UnlistenFn[] };
    const cleaned: string[] = [];
    await assert.rejects(
      () =>
        startBridgeAtomic(
          [
            async () => () => cleaned.push("u1"),
            async () => () => cleaned.push("u2"),
            async () => {
              throw new Error("listen failed");
            },
            async () => () => cleaned.push("u4"),
          ],
          state,
        ),
      /listen failed/,
    );
    assert.equal(state.started, false);
    assert.equal(state.unsubs.length, 0);
    assert.deepEqual(cleaned, ["u1", "u2"]);
  });

  it("allows retry after partial failure (not stuck started)", async () => {
    const state = { started: false, unsubs: [] as UnlistenFn[] };
    let failOnce = true;
    const arms = () => [
      async () => () => {},
      async () => {
        if (failOnce) {
          failOnce = false;
          throw new Error("transient");
        }
        return () => {};
      },
    ];
    await assert.rejects(() => startBridgeAtomic(arms(), state), /transient/);
    assert.equal(state.started, false);
    await startBridgeAtomic(arms(), state);
    assert.equal(state.started, true);
    assert.equal(state.unsubs.length, 2);
  });
});
