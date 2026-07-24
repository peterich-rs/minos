import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { planConnectionToast } from "./connection-toast-policy.ts";

describe("planConnectionToast", () => {
  it("ignores the first observation", () => {
    assert.deepEqual(
      planConnectionToast({
        prev: null,
        connected: true,
        pendingDisconnect: false,
        disconnectMessage: "down",
        connectedDetail: "up",
      }),
      { type: "none" },
    );
  });

  it("schedules disconnect when leaving connected", () => {
    assert.deepEqual(
      planConnectionToast({
        prev: true,
        connected: false,
        pendingDisconnect: false,
        disconnectMessage: "gone",
        connectedDetail: "up",
      }),
      { type: "schedule_disconnect", message: "gone" },
    );
  });

  it("does not re-schedule while disconnect is pending", () => {
    assert.deepEqual(
      planConnectionToast({
        prev: true,
        connected: false,
        pendingDisconnect: true,
        disconnectMessage: "gone",
        connectedDetail: "up",
      }),
      { type: "none" },
    );
  });

  it("cancels pending disconnect when reconnecting before toast", () => {
    assert.deepEqual(
      planConnectionToast({
        prev: true,
        connected: true,
        pendingDisconnect: true,
        disconnectMessage: "gone",
        connectedDetail: "Ready",
      }),
      { type: "cancel_pending" },
    );
  });

  it("toasts connected after a committed disconnect", () => {
    assert.deepEqual(
      planConnectionToast({
        prev: false,
        connected: true,
        pendingDisconnect: false,
        disconnectMessage: "gone",
        connectedDetail: "Managed process ready",
      }),
      { type: "toast_connected", detail: "Managed process ready" },
    );
  });
});
