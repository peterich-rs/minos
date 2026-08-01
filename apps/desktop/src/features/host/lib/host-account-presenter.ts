/**
 * Pure presenter for Host page account + link chrome (T-ui-06 / T-host-04).
 *
 * Sign-in is owned by the root LoginPage gate. Host only shows identity +
 * Link / Unlink after AppShell is entered.
 *
 * States: Signed out (defensive) / Signed in Local only / Linked / Error
 */

export type AccountAuthMode = "login" | "register";

export type HostAccountInput = {
  /** Minos session present. */
  signedIn: boolean;
  email: string | null;
  /** Daemon usable for local coding. */
  daemonReady: boolean;
  /** Host Link active (account ↔ this Mac binding). */
  relayLinked: boolean;
  /**
   * Live hub `/ws/host` (IM device online). `undefined` when unknown.
   * Distinct from [relayLinked].
   */
  hubOnline?: boolean;
  hostDisplayName: string | null;
  /** In-flight sign-in / link / unlink. */
  busy: boolean;
  /** Last user-visible error (auth or link). */
  error: string | null;
  /** Backend + Supabase env present enough to attempt cloud auth/link. */
  cloudConfigured: boolean;
};

export type HostAccountViewModel = {
  /** Primary product status for the Account / Remote section. */
  statusKind: "signed_out" | "local_only" | "linked" | "error";
  statusLabel: string;
  statusHint: string;
  emailLabel: string | null;
  showSignOut: boolean;
  /** Primary CTA for Host Link when signed in + daemon ready + not linked. */
  showLinkCta: boolean;
  linkCtaLabel: string;
  linkCtaDisabled: boolean;
  linkCtaDisabledReason: string | null;
  showUnlink: boolean;
  unlinkDisabled: boolean;
  errorMessage: string | null;
};

export function presentHostAccount(
  input: HostAccountInput,
): HostAccountViewModel {
  const email = input.email?.trim() || null;
  const errorMessage = input.error?.trim() || null;

  // Root gate should prevent AppShell without a session; keep a defensive
  // empty state if store is briefly empty after sign-out.
  if (!input.signedIn) {
    return {
      statusKind: errorMessage ? "error" : "signed_out",
      statusLabel: errorMessage ? "Sign-in error" : "Signed out",
      statusHint:
        "You will return to the sign-in screen. Sign in again to manage Host Link.",
      emailLabel: null,
      showSignOut: false,
      showLinkCta: false,
      linkCtaLabel: "Link this Mac",
      linkCtaDisabled: true,
      linkCtaDisabledReason: "Sign in first",
      showUnlink: false,
      unlinkDisabled: true,
      errorMessage,
    };
  }

  if (input.relayLinked) {
    const hubHint =
      input.hubOnline === true
        ? "Hub online — Mobile sees this device as online."
        : input.hubOnline === false
          ? "Hub offline — linked but no live /ws/host yet; Mobile shows device offline."
          : "Linked. Hub online appears when the daemon holds a live /ws/host.";
    return {
      statusKind: errorMessage ? "error" : "linked",
      statusLabel:
        input.hubOnline === true
          ? "Linked · Hub online"
          : input.hubOnline === false
            ? "Linked · Hub offline"
            : "Linked",
      statusHint: hubHint,
      emailLabel: email,
      showSignOut: true,
      showLinkCta: false,
      linkCtaLabel: "Link this Mac",
      linkCtaDisabled: true,
      linkCtaDisabledReason: null,
      showUnlink: true,
      unlinkDisabled: input.busy || !input.cloudConfigured,
      errorMessage,
    };
  }

  // Signed in, not linked → Local only (or blocked by daemon/cloud).
  const canLink =
    input.daemonReady && input.cloudConfigured && !input.busy;
  let disabledReason: string | null = null;
  if (!input.daemonReady) disabledReason = "Local daemon is offline";
  else if (!input.cloudConfigured) disabledReason = "Backend URL not configured";
  else if (input.busy) disabledReason = "Working…";

  return {
    statusKind: errorMessage ? "error" : "local_only",
    statusLabel: "Local only",
    statusHint:
      "Signed in. Link this Mac so phone and web can route to your daemon.",
    emailLabel: email,
    showSignOut: true,
    showLinkCta: true,
    linkCtaLabel: input.busy ? "Linking…" : "Link this Mac",
    linkCtaDisabled: !canLink,
    linkCtaDisabledReason: canLink ? null : disabledReason,
    showUnlink: false,
    unlinkDisabled: true,
    errorMessage,
  };
}
