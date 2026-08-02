import assert from "node:assert/strict";
import { afterEach, describe, it } from "node:test";
import {
  EMPTY_HOST_BIND,
  clearStoredHostBind,
  isAccessTokenFresh,
  loadStoredHostBind,
  saveStoredHostBind,
  sessionFromAuthResponse,
  type HostBindState,
} from "./account-session.ts";

const memory = new Map<string, string>();

function installMemoryStorage(): void {
  const storage = {
    getItem(key: string) {
      return memory.has(key) ? (memory.get(key) as string) : null;
    },
    setItem(key: string, value: string) {
      memory.set(key, String(value));
    },
    removeItem(key: string) {
      memory.delete(key);
    },
    clear() {
      memory.clear();
    },
    key(_index: number) {
      return null;
    },
    get length() {
      return memory.size;
    },
  };
  (globalThis as unknown as { window: { localStorage: typeof storage } }).window =
    { localStorage: storage };
}

afterEach(() => {
  memory.clear();
  delete (globalThis as unknown as { window?: unknown }).window;
});

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
    assert.equal(isAccessTokenFresh(base, 1_840_000), false);
  });
});

describe("account-scoped host bind storage", () => {
  const bound: HostBindState = {
    bound: true,
    hostInstallationId: "host-a",
    hostDisplayName: "This Mac",
    boundAtMs: 42,
    pairId: "pair-a",
  };

  it("isolates slots per accountId", () => {
    installMemoryStorage();
    saveStoredHostBind("acc-1", bound);
    saveStoredHostBind("acc-2", {
      ...bound,
      hostInstallationId: "host-b",
      pairId: "pair-b",
    });
    assert.equal(loadStoredHostBind("acc-1").hostInstallationId, "host-a");
    assert.equal(loadStoredHostBind("acc-2").hostInstallationId, "host-b");
    clearStoredHostBind("acc-1");
    assert.equal(loadStoredHostBind("acc-1").bound, false);
    assert.equal(loadStoredHostBind("acc-2").hostInstallationId, "host-b");
  });

  it("returns empty without account id", () => {
    installMemoryStorage();
    saveStoredHostBind("acc-1", bound);
    assert.equal(loadStoredHostBind(null).bound, false);
  });

  it("EMPTY_HOST_BIND starts unbound", () => {
    assert.equal(EMPTY_HOST_BIND.bound, false);
  });
});
