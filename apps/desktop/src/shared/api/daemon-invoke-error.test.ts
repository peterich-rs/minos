import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { DaemonInvokeError } from "./daemon-invoke-error.ts";

describe("DaemonInvokeError", () => {
  it("captures message, command, and cause", () => {
    const cause = new Error("boom");
    const err = new DaemonInvokeError("failed", "daemon_status", cause);
    assert.equal(err.name, "DaemonInvokeError");
    assert.equal(err.message, "failed");
    assert.equal(err.command, "daemon_status");
    assert.equal(err.cause, cause);
    assert.ok(err instanceof Error);
    assert.ok(err instanceof DaemonInvokeError);
  });

  it("allows omitted cause", () => {
    const err = new DaemonInvokeError("not running in Tauri", "daemon_connect");
    assert.equal(err.message, "not running in Tauri");
    assert.equal(err.command, "daemon_connect");
    assert.equal(err.cause, undefined);
  });
});
