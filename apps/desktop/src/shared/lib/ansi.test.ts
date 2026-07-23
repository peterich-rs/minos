import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { stripAnsiEscapes } from "./ansi.ts";

describe("stripAnsiEscapes", () => {
  it("strips node:test color markers", () => {
    const raw =
      "\u001b[31m✖ src/lib/stick-to-bottom.test.ts \u001b[90m(47.469042ms)\u001b[39m\u001b[39m";
    const clean = stripAnsiEscapes(raw);
    assert.equal(clean.includes("["), false);
    assert.ok(clean.includes("stick-to-bottom.test.ts"));
    assert.ok(clean.includes("47.469042ms"));
  });

  it("leaves plain text alone", () => {
    assert.equal(stripAnsiEscapes("hello +1/-2"), "hello +1/-2");
  });

  it("strips the FORCE_COLOR warning context stack dim codes", () => {
    const raw =
      "\u001b[90m    at Object.getPackageJSONURL (node:internal/modules/package_json_reader:301:9)\u001b[39m";
    assert.equal(
      stripAnsiEscapes(raw),
      "    at Object.getPackageJSONURL (node:internal/modules/package_json_reader:301:9)",
    );
  });
});
