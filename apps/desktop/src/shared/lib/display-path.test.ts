import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  collapseHomePrefix,
  formatDisplayPath,
  formatToolTarget,
  looksLikeFilePath,
} from "./display-path.ts";

describe("collapseHomePrefix", () => {
  it("maps /Users/<name> to ~", () => {
    assert.equal(
      collapseHomePrefix("/Users/fannnzhang/code/github.com/Minos/a.ts"),
      "~/code/github.com/Minos/a.ts",
    );
  });

  it("maps /home/<name> to ~", () => {
    assert.equal(collapseHomePrefix("/home/alice/proj/x"), "~/proj/x");
  });

  it("leaves relative paths alone", () => {
    assert.equal(collapseHomePrefix("apps/desktop/src/a.ts"), "apps/desktop/src/a.ts");
  });
});

describe("formatDisplayPath", () => {
  it("uses ~ and keeps short paths complete", () => {
    assert.equal(
      formatDisplayPath(
        "/Users/fannnzhang/code/github.com/Minos/README.md",
      ),
      "~/code/github.com/Minos/README.md",
    );
  });

  it("prefers the trailing path when deep", () => {
    const p = formatDisplayPath(
      "/Users/fannnzhang/code/github.com/Minos/apps/desktop/src/shared/ui/MarkdownText.tsx",
      { maxSegments: 4 },
    );
    assert.equal(p, "~/…/src/shared/ui/MarkdownText.tsx");
  });

  it("defaults to a generous segment budget so mid paths fit", () => {
    const p = formatDisplayPath(
      "/Users/fannnzhang/code/github.com/Minos/apps/desktop/src/store/workspace/helpers.ts",
    );
    // 6 segments under ~: apps/desktop/src/store/workspace/helpers.ts (6)
    // full under ~ has more: code/github.com/Minos/apps/desktop/src/store/workspace/helpers.ts
    assert.ok(p.startsWith("~/"));
    assert.ok(p.endsWith("helpers.ts"));
    assert.ok(!p.includes("/Users/"));
  });
});

describe("formatToolTarget", () => {
  it("shortens path targets", () => {
    const t = formatToolTarget(
      "/Users/me/code/repo/apps/desktop/src/features/work/SessionsView.tsx",
      { maxSegments: 3 },
    );
    assert.equal(t, "~/…/features/work/SessionsView.tsx");
  });

  it("leaves plain commands alone except home collapse", () => {
    assert.equal(formatToolTarget("cargo test -p minos"), "cargo test -p minos");
  });
});

describe("looksLikeFilePath", () => {
  it("detects absolute and relative paths", () => {
    assert.equal(looksLikeFilePath("/tmp/a.ts"), true);
    assert.equal(looksLikeFilePath("src/main.rs"), true);
    assert.equal(looksLikeFilePath("cargo test"), false);
  });
});
