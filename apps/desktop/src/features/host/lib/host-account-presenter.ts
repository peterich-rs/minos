/**
 * Pure presenter for Host page account + link chrome (T-ui-06 / T-host-04).
 *
 * States: Signed out / Signed in Local only / Linked / Error
 */

export type AccountAuthMode = "login" | "register";

export type HostAccountInput = {
  /** Minos session present. */
  signedIn: boolean;
  email: string | null;
  /** Daemon usable for local coding. */
  daemonReady: boolean;
  /** Host Link active (relay / backend collaboration). */
  relayLinked: boolean;
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
  showSignInForm: boolean;
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

  if (!input.signedIn) {
    return {
      statusKind: errorMessage ? "error" : "signed_out",
      statusLabel: errorMessage ? "Sign-in error" : "Signed out",
      statusHint: input.cloudConfigured
        ? "Sign in with your Minos account to link this Mac for phone control."
        : "Set VITE_MINOS_BACKEND_URL (and optional Supabase) to enable cloud account.",
      emailLabel: null,
      showSignInForm: true,
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
    return {
      statusKind: errorMessage ? "error" : "linked",
      statusLabel: "Linked",
      statusHint: "Remote clients can reach this Mac through your account.",
      emailLabel: email,
      showSignInForm: false,
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
    showSignInForm: false,
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
