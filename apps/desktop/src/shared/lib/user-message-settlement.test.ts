import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { deliveryStatusAfterUserSettlement } from "./user-message-settlement.ts";

describe("deliveryStatusAfterUserSettlement", () => {
  it("acked → sent", () => {
    assert.equal(deliveryStatusAfterUserSettlement("acked"), "sent");
  });

  it("timeout → sending (never false sent)", () => {
    assert.equal(deliveryStatusAfterUserSettlement("timeout"), "sending");
  });

  it("failed_terminal → failed", () => {
    assert.equal(deliveryStatusAfterUserSettlement("failed_terminal"), "failed");
  });
});
