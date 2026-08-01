import { useState, type FormEvent } from "react";
import { Sparkles } from "lucide-react";
import { useAccountStore, type AuthMode } from "@/store/account-store";
import { cn } from "@/shared/lib/utils";

/**
 * Full-screen account gate (Supabase IdP → Minos exchange).
 * Shown before AppShell when there is no valid Minos session.
 */
export function LoginPage() {
  const authMode = useAccountStore((s) => s.authMode);
  const setAuthMode = useAccountStore((s) => s.setAuthMode);
  const busy = useAccountStore((s) => s.busy);
  const error = useAccountStore((s) => s.error);
  const clearError = useAccountStore((s) => s.clearError);
  const signIn = useAccountStore((s) => s.signIn);
  const signUp = useAccountStore((s) => s.signUp);
  const isSupabaseReady = useAccountStore((s) => s.isSupabaseReady);

  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");

  const passwordReady = password.length >= 8;
  const confirmReady = password === confirmPassword && passwordReady;
  const formDisabled =
    busy ||
    !email.includes("@") ||
    !passwordReady ||
    (authMode === "register" && !confirmReady) ||
    !isSupabaseReady();

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (formDisabled) return;
    if (authMode === "register") {
      void signUp(email.trim(), password);
    } else {
      void signIn(email.trim(), password);
    }
  };

  const selectMode = (mode: AuthMode) => {
    setAuthMode(mode);
    clearError();
  };

  return (
    <div className="relative flex h-full min-h-full w-full flex-col items-center justify-center overflow-hidden px-6">
      <div className="minos-theme-gradient" aria-hidden />
      <div className="minos-theme-grain" aria-hidden />

      <div className="relative z-10 w-full max-w-sm rounded-2xl border border-ink/8 bg-surface/95 px-6 py-8 shadow-shell backdrop-blur-md">
        <div className="mb-6 flex flex-col items-center text-center">
          <div className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-ink text-surface shadow-md">
            <Sparkles className="h-6 w-6" strokeWidth={2} />
          </div>
          <h1 className="text-lg font-semibold tracking-tight text-ink">
            Minos
          </h1>
          <p className="mt-1.5 text-sm text-ink-muted">
            {authMode === "register"
              ? "Create an account to use this host console"
              : "Sign in to continue"}
          </p>
        </div>

        <div className="mb-4 grid grid-cols-2 gap-1 rounded-lg bg-surface-muted p-1">
          {(["login", "register"] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              onClick={() => selectMode(mode)}
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

        <form className="space-y-3" onSubmit={onSubmit}>
          <label className="block space-y-1">
            <span className="text-3xs font-medium text-ink-muted">Email</span>
            <input
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              type="email"
              autoComplete="email"
              required
              autoFocus
              className="h-10 w-full rounded-lg border border-ink/10 bg-surface px-3 text-sm text-ink outline-none ring-accent/30 placeholder:text-ink-muted focus:ring-2"
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
                authMode === "login" ? "current-password" : "new-password"
              }
              required
              className="h-10 w-full rounded-lg border border-ink/10 bg-surface px-3 text-sm text-ink outline-none ring-accent/30 placeholder:text-ink-muted focus:ring-2"
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
                autoComplete="new-password"
                required
                className="h-10 w-full rounded-lg border border-ink/10 bg-surface px-3 text-sm text-ink outline-none ring-accent/30 focus:ring-2"
                placeholder="Again"
              />
            </label>
          ) : null}

          {error ? (
            <div className="rounded-lg bg-status-failed/10 px-3 py-2 text-2xs text-status-failed ring-1 ring-inset ring-status-failed/25">
              {error}
            </div>
          ) : null}

          {!isSupabaseReady() ? (
            <div className="rounded-lg bg-status-failed/10 px-3 py-2 text-2xs text-status-failed ring-1 ring-inset ring-status-failed/25">
              Set{" "}
              <span className="font-mono">VITE_SUPABASE_URL</span> and{" "}
              <span className="font-mono">VITE_SUPABASE_ANON_KEY</span> (and
              backend URL) to sign in.
            </div>
          ) : null}

          <button
            type="submit"
            disabled={formDisabled}
            className="flex h-10 w-full items-center justify-center rounded-lg bg-accent text-sm font-semibold text-white transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            {busy
              ? "Working…"
              : authMode === "login"
                ? "Sign in"
                : "Create account"}
          </button>

          <p className="text-center text-3xs leading-snug text-ink-muted">
            Supabase Auth → Minos session. Link this Mac later on the Host page
            for phone control.
          </p>
        </form>
      </div>
    </div>
  );
}
