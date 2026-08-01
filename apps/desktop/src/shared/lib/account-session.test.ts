import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  EMPTY_HOST_LINK,
  isAccessTokenFresh,
  sessionFromAuthResponse,
} from "./account-session.ts";

describe("sessionFromAuthResponse", () => {
  it("maps AuthResp into MinosSession", () => {
    const session = sessionFromAuthResponse({
      account: { account_id: "acc-1", email: "a@b.co" },
      access_token: "at",
      refresh_token: "rt",
      expires_in: 900,
      issuedAtMs: 1_700_000_000_000,
    });
    assert.equal(session.accountId, "acc-1");
    assert.equal(session.email, "a@b.co");
    assert.equal(session.accessToken, "at");
    assert.equal(session.refreshToken, "rt");
    assert.equal(session.expiresInSec, 900);
    assert.equal(session.issuedAtMs, 1_700_000_000_000);
  });
});

describe("isAccessTokenFresh", () => {
  const base = {
    accountId: "a",
    email: "e",
    accessToken: "t",
    refreshToken: "r",
    issuedAtMs: 1_000_000,
    expiresInSec: 900,
  };

  it("true well before expiry", () => {
    assert.equal(isAccessTokenFresh(base, 1_000_000 + 60_000), true);
  });

  it("false inside 60s skew of expiry", () => {
    // expires at 1_000_000 + 900_000 = 1_900_000; skew → 1_840_000
    assert.equal(isAccessTokenFresh(base, 1_840_000), false);
  });
});

describe("EMPTY_HOST_LINK", () => {
  it("starts unlinked", () => {
    assert.equal(EMPTY_HOST_LINK.linked, false);
    assert.equal(EMPTY_HOST_LINK.hostInstallationId, null);
  });
});
