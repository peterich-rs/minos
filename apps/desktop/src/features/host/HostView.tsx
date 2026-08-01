import { useMemo, useState, type ReactNode } from "react";
import {
  Check,
  ChevronDown,
  Circle,
  Download,
  Link2,
  LogOut,
  Palette,
  RefreshCw,
  Server,
  Unlink,
  UserRound,
} from "lucide-react";
import { useWorkspaceStore } from "@/store/workspace-store";
import { useAccountStore } from "@/store/account-store";
import {
  PROJECT_HOST_THIS_MAC,
  deriveHostPresence,
  presenceDotClass,
} from "@/shared/lib/host-status";
import { cn } from "@/shared/lib/utils";
import {
  ACCENT_COLORS,
  NEUTRAL_ACCENT,
  useTheme,
} from "@/shared/theme/ThemeProvider";
import { THEME_LABELS, type SyntaxThemeName } from "@/shared/theme/theme-loader";
import { UpdateChecker } from "@/features/settings/UpdateChecker";
import {
  PageHeader,
  PageHeaderPrimaryButton,
} from "@/shared/ui/PageHeader";
import { presentHostAccount } from "@/features/host/lib/host-account-presenter";
import { backendHttpBase } from "@/shared/lib/minos-cloud";

/**
 * Shared Host settings card.
 * Match the Appearance card look: same surface as the page stack (not raised),
 * hairline inset ring only — no heavy border, no muted header stripe.
 */
const hostCardClass =
  "overflow-hidden rounded-2xl border border-ink/6 bg-surface shadow-panel";

const hostCardHeaderClass =
  "flex items-center gap-2 border-b border-ink/6 px-3.5 py-2.5";

export function HostView() {
  const connection = useWorkspaceStore((s) => s.connection);
  const source = useWorkspaceStore((s) => s.source);
  const error = useWorkspaceStore((s) => s.error);
  const actionError = useWorkspaceStore((s) => s.actionError);
  const bootstrap = useWorkspaceStore((s) => s.bootstrap);
  const [diagOpen, setDiagOpen] = useState(false);
  const {
    themeName,
    themes,
    setTheme,
    accentColor,
    setAccentColor,
    isDark,
    isLoading: themeLoading,
  } = useTheme();

  const session = useAccountStore((s) => s.session);
  const hostLink = useAccountStore((s) => s.hostLink);
  const authMode = useAccountStore((s) => s.authMode);
  const setAuthMode = useAccountStore((s) => s.setAuthMode);
  const accountBusy = useAccountStore((s) => s.busy);
  const accountError = useAccountStore((s) => s.error);
  const clearError = useAccountStore((s) => s.clearError);
  const signIn = useAccountStore((s) => s.signIn);
  const signUp = useAccountStore((s) => s.signUp);
  const signOut = useAccountStore((s) => s.signOut);
  const linkThisMac = useAccountStore((s) => s.linkThisMac);
  const unlinkThisMac = useAccountStore((s) => s.unlinkThisMac);
  const isSupabaseReady = useAccountStore((s) => s.isSupabaseReady);
  const isCloudConfigured = useAccountStore((s) => s.isCloudConfigured);

  const daemonReady =
    source === "daemon" && connection?.connected === true;
  const relayLinked = hostLink.linked === true;

  const presence = deriveHostPresence({
    source,
    daemonConnected: daemonReady,
    relayLinked,
  });

  const accountVm = useMemo(
    () =>
      presentHostAccount({
        signedIn: session != null,
        email: session?.email ?? null,
        daemonReady,
        relayLinked,
        hostDisplayName: hostLink.hostDisplayName,
        busy: accountBusy,
        error: accountError,
        cloudConfigured: isCloudConfigured(),
      }),
    [
      session,
      daemonReady,
      relayLinked,
      hostLink.hostDisplayName,
      accountBusy,
      accountError,
      isCloudConfigured,
    ],
  );

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");

  const passwordReady = password.length >= 8;
  const confirmReady = password === confirmPassword && passwordReady;
  const formDisabled =
    accountBusy ||
    !email.includes("@") ||
    !passwordReady ||
    (authMode === "register" && !confirmReady);

  const lastError = connection?.error || error || actionError || null;
  const processLabel = !presence.runtimeReady
    ? "Not connected"
    : connection?.managed
      ? "In-process"
      : "External";

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-canvas-soft/40">
      <PageHeader
        title="Host"
        description={`${PROJECT_HOST_THIS_MAC} · local coding works without remote pairing`}
        badge={
          <span
            className={cn(
              "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-2xs font-medium",
              presence.tone === "ready" && "bg-status-done/15 text-status-done",
              presence.tone === "unavailable" &&
                "bg-status-failed/15 text-status-failed",
              presence.tone === "preview" &&
                "bg-status-running/15 text-status-running",
            )}
          >
            <Circle
              className={cn(
                "h-2 w-2 fill-current",
                presenceDotClass(presence.tone),
              )}
            />
            {presence.label}
          </span>
        }
        action={
          <PageHeaderPrimaryButton onClick={() => void bootstrap()}>
            <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
            {presence.runtimeReady ? "Reconnect" : "Connect"}
          </PageHeaderPrimaryButton>
        }
      />

      <div className="scrollbar-thin flex-1 overflow-y-auto">
        <div className="mx-auto max-w-xl space-y-3 px-5 py-5 sm:px-6">
          <section className={hostCardClass}>
            <div className={hostCardHeaderClass}>
              <Server className="h-3.5 w-3.5 text-ink-muted" strokeWidth={1.8} />
              <h2 className="text-xs font-semibold text-ink">Runtime</h2>
            </div>
            <dl className="divide-y divide-ink/[0.05]">
              <Row label="Machine" value={PROJECT_HOST_THIS_MAC} />
              <Row
                label="Status"
                value={
                  <span
                    className={cn(
                      "font-medium",
                      presence.tone === "ready" && "text-status-done",
                      presence.tone === "unavailable" && "text-status-failed",
                      presence.tone === "preview" && "text-status-running",
                    )}
                  >
                    {presence.readinessLabel}
                  </span>
                }
              />
              <Row
                label="Link"
                value={
                  <span className="inline-flex items-center gap-1.5">
                    <Link2 className="h-3 w-3 text-ink-muted" strokeWidth={2} />
                    {presence.linkLabel}
                  </span>
                }
                hint={
                  presence.linkMode === "linked"
                    ? "Remote clients can reach this Mac"
                    : "Backend not linked — phone control unavailable"
                }
              />
              <Row
                label="Process"
                value={processLabel}
                mono={presence.runtimeReady}
              />
            </dl>
          </section>

          <section className={hostCardClass}>
            <div className={cn(hostCardHeaderClass, "justify-between")}>
              <div className="flex min-w-0 items-center gap-2">
                <UserRound
                  className="h-3.5 w-3.5 shrink-0 text-ink-muted"
                  strokeWidth={1.8}
                />
                <h2 className="text-xs font-semibold text-ink">
                  Account &amp; remote
                </h2>
              </div>
              <span
                className={cn(
                  "shrink-0 rounded-md px-1.5 py-0.5 text-3xs font-medium uppercase tracking-wide",
                  accountVm.statusKind === "linked" &&
                    "bg-status-done/15 text-status-done",
                  accountVm.statusKind === "local_only" &&
                    "bg-surface-muted text-ink-muted",
                  accountVm.statusKind === "signed_out" &&
                    "bg-surface-muted text-ink-muted",
                  accountVm.statusKind === "error" &&
                    "bg-status-failed/15 text-status-failed",
                )}
              >
                {accountVm.statusLabel}
              </span>
            </div>

            <div className="space-y-3 px-3.5 py-3">
              <p className="text-2xs leading-snug text-ink-muted">
                {accountVm.statusHint}
              </p>

              {accountVm.emailLabel ? (
                <div className="flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <p className="text-2xs font-medium text-ink-muted">
                      Signed in
                    </p>
                    <p className="truncate text-sm font-medium text-ink">
                      {accountVm.emailLabel}
                    </p>
                  </div>
                  {accountVm.showSignOut ? (
                    <button
                      type="button"
                      disabled={accountBusy}
                      onClick={() => void signOut()}
                      className="inline-flex shrink-0 items-center gap-1 rounded-lg bg-surface-muted px-2.5 py-1.5 text-2xs font-semibold text-ink-secondary transition-colors hover:bg-surface-hover hover:text-ink disabled:opacity-50"
                    >
                      <LogOut className="h-3 w-3" strokeWidth={2} />
                      Sign out
                    </button>
                  ) : null}
                </div>
              ) : null}

              {accountVm.showSignInForm ? (
                <div className="space-y-2.5">
                  <div className="grid grid-cols-2 gap-1 rounded-lg bg-surface-muted p-1">
                    {(["login", "register"] as const).map((mode) => (
                      <button
                        key={mode}
                        type="button"
                        onClick={() => {
                          setAuthMode(mode);
                          clearError();
                        }}
                        className={cn(
                          "rounded-md py-1.5 text-2xs font-medium transition-colors",
                          authMode === mode
                            ? "bg-surface text-ink shadow-sm"
                            : "text-ink-muted hover:text-ink",
                        )}
                      >
                        {mode === "login" ? "Sign in" : "Register"}
                      </button>
                    ))}
                  </div>
                  <form
                    className="space-y-2"
                    onSubmit={(e) => {
                      e.preventDefault();
                      if (formDisabled) return;
                      if (authMode === "register") {
                        void signUp(email.trim(), password);
                      } else {
                        void signIn(email.trim(), password);
                      }
                    }}
                  >
                    <label className="block space-y-1">
                      <span className="text-3xs font-medium text-ink-muted">
                        Email
                      </span>
                      <input
                        value={email}
                        onChange={(e) => setEmail(e.target.value)}
                        type="email"
                        autoComplete="email"
                        required
                        className="h-9 w-full rounded-lg border border-ink/10 bg-surface px-2.5 text-sm text-ink outline-none ring-accent/30 placeholder:text-ink-muted focus:ring-2"
                        placeholder="you@example.com"
                      />
                    </label>
                    <label className="block space-y-1">
                      <span className="text-3xs font-medium text-ink-muted">
                        Password
                      </span>
                      <input
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        type="password"
                        autoComplete={
                          authMode === "login"
                            ? "current-password"
                            : "new-password"
                        }
                        required
                        className="h-9 w-full rounded-lg border border-ink/10 bg-surface px-2.5 text-sm text-ink outline-none ring-accent/30 placeholder:text-ink-muted focus:ring-2"
                        placeholder="At least 8 characters"
                      />
                    </label>
                    {authMode === "register" ? (
                      <label className="block space-y-1">
                        <span className="text-3xs font-medium text-ink-muted">
                          Confirm password
                        </span>
                        <input
                          value={confirmPassword}
                          onChange={(e) => setConfirmPassword(e.target.value)}
                          type="password"
                          required
                          className="h-9 w-full rounded-lg border border-ink/10 bg-surface px-2.5 text-sm text-ink outline-none ring-accent/30 focus:ring-2"
                          placeholder="Again"
                        />
                      </label>
                    ) : null}
                    <button
                      type="submit"
                      disabled={formDisabled}
                      className="flex h-9 w-full items-center justify-center rounded-lg bg-accent text-2xs font-semibold text-white transition-opacity hover:opacity-90 disabled:opacity-40"
                    >
                      {accountBusy
                        ? "Working…"
                        : authMode === "login"
                          ? isSupabaseReady()
                            ? "Sign in (Supabase → Minos)"
                            : "Sign in"
                          : isSupabaseReady()
                            ? "Create account (Supabase → Minos)"
                            : "Create account"}
                    </button>
                    <p className="text-3xs text-ink-muted">
                      {isSupabaseReady()
                        ? "IdP via Supabase; product session is Minos JWT."
                        : "Minos password login (set Supabase env for IdP)."}
                    </p>
                  </form>
                </div>
              ) : null}

              {accountVm.showLinkCta ? (
                <div className="flex flex-col gap-2 border-t border-ink/[0.06] pt-3 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-ink">Link this Mac</p>
                    <p className="mt-0.5 text-2xs leading-snug text-ink-muted">
                      Proves local daemon control and binds this host to your
                      account. No QR code.
                    </p>
                    {accountVm.linkCtaDisabledReason ? (
                      <p className="mt-1 text-3xs text-ink-muted">
                        {accountVm.linkCtaDisabledReason}
                      </p>
                    ) : null}
                  </div>
                  <button
                    type="button"
                    disabled={accountVm.linkCtaDisabled}
                    onClick={() => void linkThisMac(PROJECT_HOST_THIS_MAC)}
                    className="inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg bg-accent px-3 py-2 text-2xs font-semibold text-white transition-opacity hover:opacity-90 disabled:opacity-40"
                  >
                    <Link2 className="h-3.5 w-3.5" strokeWidth={2} />
                    {accountVm.linkCtaLabel}
                  </button>
                </div>
              ) : null}

              {accountVm.showUnlink ? (
                <div className="flex flex-col gap-2 border-t border-ink/[0.06] pt-3 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-ink">
                      {hostLink.hostDisplayName ?? PROJECT_HOST_THIS_MAC}
                    </p>
                    <p className="mt-0.5 text-2xs leading-snug text-ink-muted">
                      Linked to your account
                      {hostLink.linkedAtMs
                        ? ` · ${new Date(hostLink.linkedAtMs).toLocaleString()}`
                        : ""}
                    </p>
                  </div>
                  <button
                    type="button"
                    disabled={accountVm.unlinkDisabled}
                    onClick={() => void unlinkThisMac()}
                    className="inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg bg-surface-muted px-3 py-2 text-2xs font-semibold text-ink-secondary transition-colors hover:bg-surface-hover hover:text-ink disabled:opacity-40"
                  >
                    <Unlink className="h-3.5 w-3.5" strokeWidth={2} />
                    Unlink
                  </button>
                </div>
              ) : null}

              {accountVm.errorMessage ? (
                <div className="rounded-lg bg-status-failed/10 px-3 py-2 text-2xs text-status-failed ring-1 ring-inset ring-status-failed/25">
                  {accountVm.errorMessage}
                </div>
              ) : null}
            </div>
          </section>

          <section className={hostCardClass}>
            <div className={hostCardHeaderClass}>
              <Download
                className="h-3.5 w-3.5 text-ink-muted"
                strokeWidth={1.8}
              />
              <h2 className="text-xs font-semibold text-ink">Updates</h2>
            </div>
            <div className="px-3.5 py-3">
              <UpdateChecker />
            </div>
          </section>

          <section className={hostCardClass}>
            <div className={hostCardHeaderClass}>
              <Palette
                className="h-3.5 w-3.5 text-ink-muted"
                strokeWidth={1.8}
              />
              <h2 className="text-xs font-semibold text-ink">Appearance</h2>
              {themeLoading ? (
                <span className="ml-auto text-3xs text-ink-muted">
                  Applying…
                </span>
              ) : (
                <span className="ml-auto text-3xs text-ink-muted">
                  {isDark ? "Dark" : "Light"}
                </span>
              )}
            </div>
            <div className="space-y-3.5 px-3.5 py-3">
              <div>
                <p className="mb-2 text-2xs font-medium text-ink-muted">
                  Theme
                </p>
                <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-3">
                  {themes.map((name) => {
                    const selected = themeName === name;
                    return (
                      <button
                        key={name}
                        type="button"
                        onClick={() => setTheme(name)}
                        aria-pressed={selected}
                        className={cn(
                          "flex items-center justify-between gap-1.5 rounded-lg px-2.5 py-2 text-left text-2xs font-medium transition-colors",
                          selected
                            ? "bg-accent/15 text-ink"
                            : "bg-surface-muted/50 text-ink-secondary hover:bg-surface-hover hover:text-ink",
                        )}
                      >
                        <span className="min-w-0 truncate">
                          {THEME_LABELS[name as SyntaxThemeName] ?? name}
                        </span>
                        {selected ? (
                          <Check
                            className="h-3 w-3 shrink-0 text-accent-strong"
                            strokeWidth={2.5}
                            aria-hidden
                          />
                        ) : null}
                      </button>
                    );
                  })}
                </div>
              </div>
              <div>
                <p className="mb-2 text-2xs font-medium text-ink-muted">
                  Accent
                </p>
                <div className="flex flex-wrap gap-2">
                  {ACCENT_COLORS.map((c) => {
                    const selected = accentColor === c.value;
                    return (
                      <button
                        key={c.value}
                        type="button"
                        title={c.name}
                        onClick={() => setAccentColor(c.value)}
                        className={cn(
                          "h-7 w-7 rounded-full transition-transform hover:scale-105",
                          selected
                            ? "ring-2 ring-accent ring-offset-2 ring-offset-surface"
                            : "ring-1 ring-ink/10 ring-offset-1 ring-offset-surface",
                        )}
                        style={{
                          background:
                            c.value === NEUTRAL_ACCENT
                              ? "linear-gradient(135deg,#a8a29e,#57534e)"
                              : c.value,
                        }}
                        aria-label={`Accent ${c.name}`}
                        aria-pressed={selected}
                      />
                    );
                  })}
                </div>
              </div>
            </div>
          </section>

          {lastError ? (
            <div className="rounded-xl bg-status-failed/10 px-3.5 py-2.5 text-xs text-status-failed ring-1 ring-inset ring-status-failed/25">
              <div className="font-semibold">Last error</div>
              <p className="mt-0.5 break-all font-mono text-2xs leading-snug opacity-90">
                {lastError}
              </p>
            </div>
          ) : null}

          <section className={hostCardClass}>
            <button
              type="button"
              onClick={() => setDiagOpen((v) => !v)}
              className="flex w-full items-center justify-between gap-2 px-3.5 py-2 text-left transition-colors hover:bg-surface-muted/40"
              aria-expanded={diagOpen}
            >
              <span className="text-xs font-semibold text-ink-secondary">
                Diagnostics
              </span>
              <ChevronDown
                className={cn(
                  "h-3.5 w-3.5 text-ink-muted transition-transform",
                  diagOpen && "rotate-180",
                )}
                strokeWidth={2}
              />
            </button>
            {diagOpen ? (
              <dl className="border-t border-ink/[0.06] divide-y divide-ink/[0.05]">
                <Row label="Data source" value={source} mono />
                <Row
                  label="Connect path"
                  value={connection?.source ?? "—"}
                  mono
                />
                <Row
                  label="Endpoint"
                  value={connection?.endpoint ?? "—"}
                  mono
                />
                <Row
                  label="Managed"
                  value={connection?.managed ? "yes" : "no"}
                  mono
                />
                <Row
                  label="Backend"
                  value={backendHttpBase()}
                  mono
                />
                <Row
                  label="Account"
                  value={session?.email || "signed out"}
                  mono
                />
                <Row
                  label="Host link"
                  value={
                    hostLink.linked
                      ? hostLink.hostInstallationId ?? "linked"
                      : "local only"
                  }
                  mono
                />
                <Row label="Last error" value={lastError ?? "—"} mono />
              </dl>
            ) : (
              <p className="border-t border-ink/[0.06] px-3.5 py-2 text-2xs text-ink-muted">
                Endpoint, process mode, connect path — expand if debugging.
              </p>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}

function Row({
  label,
  value,
  hint,
  mono,
}: {
  label: string;
  value: ReactNode;
  hint?: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-start gap-4 px-3.5 py-2 sm:gap-6">
      <dt className="w-[5.5rem] shrink-0 pt-0.5 text-2xs font-medium text-ink-muted sm:w-24">
        {label}
      </dt>
      <dd className="min-w-0 flex-1">
        <div
          className={cn(
            "break-all text-sm text-ink",
            mono && "font-mono text-xs",
          )}
        >
          {value}
        </div>
        {hint ? (
          <p className="mt-0.5 text-2xs leading-snug text-ink-muted">{hint}</p>
        ) : null}
      </dd>
    </div>
  );
}
