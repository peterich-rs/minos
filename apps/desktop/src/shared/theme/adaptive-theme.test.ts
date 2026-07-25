import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { createThemeVars, luminance } from "./adaptive-theme.ts";

describe("createThemeVars", () => {
  it("marks github-dark-like bg as dark and lifts surface-raised off pure white", () => {
    const dark = createThemeVars("#0d1117", "#e6edf3", "#8b949e");
    assert.equal(dark.isDark, true);
    assert.notEqual(dark.vars["--color-surface-raised"], "255 255 255");
    assert.ok(luminance("#0d1117") < 0.5);
  });

  it("keeps light themes with white raised surface", () => {
    const light = createThemeVars("#ffffff", "#1f2328", "#656d76");
    assert.equal(light.isDark, false);
    assert.equal(light.vars["--color-surface-raised"], "255 255 255");
  });
});
