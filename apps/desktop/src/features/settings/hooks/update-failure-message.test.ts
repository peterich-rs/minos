import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { formatUpdateFailureMessage } from "./update-failure-message.ts";

describe("formatUpdateFailureMessage", () => {
  it("returns only the install error when daemon was restored", () => {
    assert.equal(
      formatUpdateFailureMessage("signature invalid", { restored: true }),
      "signature invalid",
    );
  });

  it("appends restore failure detail when daemon did not come back", () => {
    const msg = formatUpdateFailureMessage("disk full", {
      restored: false,
      error: "bind failed",
    });
    assert.match(msg, /disk full/);
    assert.match(msg, /bind failed/);
    assert.match(msg, /restore failed/);
  });

  it("uses a default restore detail when error is empty", () => {
    const msg = formatUpdateFailureMessage("install failed", {
      restored: false,
    });
    assert.match(msg, /did not come back online/);
  });
});
