import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { hasPrimaryShortcutModifier } from "./platform.ts";

describe("hasPrimaryShortcutModifier", () => {
  it("is true for meta or ctrl", () => {
    assert.equal(
      hasPrimaryShortcutModifier({ metaKey: true, ctrlKey: false }),
      true,
    );
    assert.equal(
      hasPrimaryShortcutModifier({ metaKey: false, ctrlKey: true }),
      true,
    );
  });

  it("is false when neither is held", () => {
    assert.equal(
      hasPrimaryShortcutModifier({ metaKey: false, ctrlKey: false }),
      false,
    );
  });
});
