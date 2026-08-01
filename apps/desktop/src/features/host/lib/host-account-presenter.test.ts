import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { presentHostAccount } from "./host-account-presenter.ts";

const base = {
  signedIn: false,
  email: null as string | null,
  daemonReady: true,
  relayLinked: false,
  hostDisplayName: null as string | null,
  busy: false,
  error: null as string | null,
  cloudConfigured: true,
};

describe("presentHostAccount", () => {
  it("signed out shows sign-in form and no link CTA", () => {
    const vm = presentHostAccount(base);
    assert.equal(vm.statusKind, "signed_out");
    assert.equal(vm.showSignInForm, true);
    assert.equal(vm.showLinkCta, false);
    assert.equal(vm.showSignOut, false);
  });

  it("signed in local only exposes Link this Mac when daemon ready", () => {
    const vm = presentHostAccount({
      ...base,
      signedIn: true,
      email: "you@example.com",
      relayLinked: false,
      daemonReady: true,
    });
    assert.equal(vm.statusKind, "local_only");
    assert.equal(vm.showLinkCta, true);
    assert.equal(vm.linkCtaDisabled, false);
    assert.equal(vm.linkCtaLabel, "Link this Mac");
    assert.equal(vm.emailLabel, "you@example.com");
    assert.equal(vm.showSignOut, true);
  });

  it("disables link when daemon offline", () => {
    const vm = presentHostAccount({
      ...base,
      signedIn: true,
      email: "you@example.com",
      daemonReady: false,
    });
    assert.equal(vm.linkCtaDisabled, true);
    assert.match(vm.linkCtaDisabledReason ?? "", /daemon/i);
  });

  it("linked shows unlink and hides link CTA", () => {
    const vm = presentHostAccount({
      ...base,
      signedIn: true,
      email: "you@example.com",
      relayLinked: true,
      hostDisplayName: "Studio Mac",
    });
    assert.equal(vm.statusKind, "linked");
    assert.equal(vm.statusLabel, "Linked");
    assert.equal(vm.showLinkCta, false);
    assert.equal(vm.showUnlink, true);
  });

  it("surfaces error kind when error present", () => {
    const vm = presentHostAccount({
      ...base,
      signedIn: true,
      email: "you@example.com",
      error: "proof_invalid",
    });
    assert.equal(vm.statusKind, "error");
    assert.equal(vm.errorMessage, "proof_invalid");
  });
});
