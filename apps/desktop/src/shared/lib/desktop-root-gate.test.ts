import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { decideDesktopRoot } from "./desktop-root-gate.ts";

describe("decideDesktopRoot", () => {
  it("shows boot while auth is hydrating", () => {
    assert.equal(
      decideDesktopRoot({ authPhase: "booting", workspaceBooting: false }),
      "boot",
    );
    assert.equal(
      decideDesktopRoot({ authPhase: "booting", workspaceBooting: true }),
      "boot",
    );
  });

  it("shows login when unauthenticated even if workspace was booting", () => {
    assert.equal(
      decideDesktopRoot({
        authPhase: "unauthenticated",
        workspaceBooting: true,
      }),
      "login",
    );
  });

  it("shows boot then app after authenticated", () => {
    assert.equal(
      decideDesktopRoot({
        authPhase: "authenticated",
        workspaceBooting: true,
      }),
      "boot",
    );
    assert.equal(
      decideDesktopRoot({
        authPhase: "authenticated",
        workspaceBooting: false,
      }),
      "app",
    );
  });
});
