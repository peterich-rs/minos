import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { planConnectionCardVisibility } from "./connection-card-policy.ts";

describe("planConnectionCardVisibility", () => {
  it("hides while booting or on mock source", () => {
    assert.equal(
      planConnectionCardVisibility({
        booting: true,
        source: "daemon",
        connected: false,
        dismissed: false,
      }),
      "hidden",
    );
    assert.equal(
      planConnectionCardVisibility({
        booting: false,
        source: "mock",
        connected: false,
        dismissed: false,
      }),
      "hidden",
    );
  });

  it("hides when connected or dismissed", () => {
    assert.equal(
      planConnectionCardVisibility({
        booting: false,
        source: "daemon",
        connected: true,
        dismissed: false,
      }),
      "hidden",
    );
    assert.equal(
      planConnectionCardVisibility({
        booting: false,
        source: "daemon",
        connected: false,
        dismissed: true,
      }),
      "hidden",
    );
  });

  it("shows when daemon is disconnected and not dismissed", () => {
    assert.equal(
      planConnectionCardVisibility({
        booting: false,
        source: "daemon",
        connected: false,
        dismissed: false,
      }),
      "show",
    );
  });
});
