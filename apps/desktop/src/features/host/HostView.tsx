import { useState, type ReactNode } from "react";
import {
  ChevronDown,
  Circle,
  Link2,
  Palette,
  QrCode,
  RefreshCw,
  Server,
} from "lucide-react";
import { useWorkspaceStore } from "@/store/workspace-store";
import {
  PROJECT_HOST_THIS_MAC,
  deriveHostPresence,
  presenceDotClass,
} from "@/shared/lib/host-status";
import { cn } from "@/shared/lib/utils";
import {
  ACCENT_COLORS,
  useTheme,
} from "@/shared/theme/ThemeProvider";
import { THEME_LABELS, type SyntaxThemeName } from "@/shared/theme/theme-loader";

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

  // v1: local daemon only; wire relayLinked when daemon exposes relay status.
  const presence = deriveHostPresence({
    source,
    daemonConnected: source === "daemon" && connection?.connected === true,
    relayLinked: false,
  });

  const lastError = connection?.error || error || actionError || null;
  const processLabel = !presence.runtimeReady
    ? "Not connected"
    : connection?.managed
      ? "In-process"
      : "External";

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <header className="flex shrink-0 items-start justify-between gap-4 border-b border-ink/5 px-5 py-4 sm:px-6">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2.5">
            <h1 className="text-[18px] font-semibold tracking-tight text-ink">
              Host
            </h1>
            <span
              className={cn(
                "inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium",
                presence.tone === "ready" &&
                  "bg-emerald-50 text-emerald-800 ring-1 ring-emerald-200/70",
                presence.tone === "unavailable" &&
                  "bg-rose-50 text-rose-800 ring-1 ring-rose-200/70",
                presence.tone === "preview" &&
                  "bg-amber-50 text-amber-900 ring-1 ring-amber-200/70",
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
          </div>
          <p className="mt-1 text-[12px] text-ink-muted">
            {PROJECT_HOST_THIS_MAC} · local coding works without remote pairing
          </p>
        </div>
        <button
          type="button"
          onClick={() => void bootstrap()}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white hover:opacity-90"
        >
          <RefreshCw className="h-3.5 w-3.5" strokeWidth={2} />
          {presence.runtimeReady ? "Reconnect" : "Connect"}
        </button>
      </header>

      <div className="scrollbar-thin flex-1 overflow-y-auto">
        <div className="mx-auto max-w-xl px-5 py-4 sm:px-6">
          {/* Single dense status block — no repeated summary cards */}
          <section className="overflow-hidden rounded-xl border border-ink/8 bg-white">
            <div className="flex items-center gap-2 border-b border-ink/5 bg-surface-muted/40 px-3.5 py-2">
              <Server className="h-3.5 w-3.5 text-ink-muted" strokeWidth={1.8} />
              <h2 className="text-[12px] font-semibold text-ink">Runtime</h2>
            </div>
            <dl className="divide-y divide-ink/5">
              <Row label="Machine" value={PROJECT_HOST_THIS_MAC} />
              <Row
                label="Status"
                value={
                  <span
                    className={cn(
                      "font-medium",
                      presence.tone === "ready" && "text-emerald-700",
                      presence.tone === "unavailable" && "text-rose-700",
                      presence.tone === "preview" && "text-amber-800",
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

          <section className="mt-3 overflow-hidden rounded-xl border border-ink/8 bg-surface">
            <div className="flex items-center gap-2 border-b border-ink/5 bg-surface-muted/40 px-3.5 py-2">
              <Palette
                className="h-3.5 w-3.5 text-ink-muted"
                strokeWidth={1.8}
              />
              <h2 className="text-[12px] font-semibold text-ink">Appearance</h2>
              {themeLoading ? (
                <span className="ml-auto text-[10px] text-ink-muted">
                  Applying…
                </span>
              ) : (
                <span className="ml-auto text-[10px] text-ink-muted">
                  {isDark ? "Dark" : "Light"}
                </span>
              )}
            </div>
            <div className="space-y-3 px-3.5 py-3">
              <div>
                <p className="mb-1.5 text-[11px] font-medium text-ink-secondary">
                  Theme (Shiki-driven)
                </p>
                <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-3">
                  {themes.map((name) => (
                    <button
                      key={name}
                      type="button"
                      onClick={() => setTheme(name)}
                      className={cn(
                        "rounded-lg border px-2 py-1.5 text-left text-[11px] font-medium transition-colors",
                        themeName === name
                          ? "border-accent bg-accent-soft text-ink ring-1 ring-accent/30"
                          : "border-ink/10 bg-surface-muted/60 text-ink-secondary hover:bg-surface-hover",
                      )}
                    >
                      {THEME_LABELS[name as SyntaxThemeName] ?? name}
                    </button>
                  ))}
                </div>
              </div>
              <div>
                <p className="mb-1.5 text-[11px] font-medium text-ink-secondary">
                  Accent
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {ACCENT_COLORS.map((c) => (
                    <button
                      key={c.value}
                      type="button"
                      title={c.name}
                      onClick={() => setAccentColor(c.value)}
                      className={cn(
                        "h-7 w-7 rounded-full border-2 transition-transform hover:scale-105",
                        accentColor === c.value
                          ? "border-ink scale-105"
                          : "border-transparent",
                      )}
                      style={{
                        background:
                          c.value === "neutral"
                            ? "linear-gradient(135deg,#a8a29e,#57534e)"
                            : c.value,
                      }}
                      aria-label={`Accent ${c.name}`}
                    />
                  ))}
                </div>
              </div>
            </div>
          </section>

          <section className="mt-3 overflow-hidden rounded-xl border border-ink/8 bg-white">
            <div className="flex items-center justify-between gap-3 border-b border-ink/5 bg-surface-muted/40 px-3.5 py-2">
              <div className="flex min-w-0 items-center gap-2">
                <QrCode
                  className="h-3.5 w-3.5 shrink-0 text-ink-muted"
                  strokeWidth={1.8}
                />
                <h2 className="text-[12px] font-semibold text-ink">Pairing</h2>
              </div>
              <span className="shrink-0 rounded-md bg-surface-muted px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-ink-muted">
                Soon
              </span>
            </div>
            <div className="flex items-center justify-between gap-3 px-3.5 py-2.5">
              <div className="min-w-0">
                <p className="text-[13px] font-medium text-ink">
                  Remote control
                </p>
                <p className="mt-0.5 text-[11px] leading-snug text-ink-muted">
                  Pair a phone or another client when link is available. Not
                  required for local projects.
                </p>
              </div>
              <button
                type="button"
                disabled
                className="shrink-0 rounded-lg border border-ink/10 bg-surface-muted px-2.5 py-1.5 text-[11px] font-semibold text-ink-muted opacity-70"
                title="Pairing lands with relay link"
              >
                Show QR
              </button>
            </div>
          </section>

          {lastError ? (
            <div className="mt-3 rounded-xl border border-rose-200/80 bg-rose-50/80 px-3.5 py-2.5 text-[12px] text-rose-900">
              <div className="font-semibold">Last error</div>
              <p className="mt-0.5 break-all font-mono text-[11px] leading-snug opacity-90">
                {lastError}
              </p>
            </div>
          ) : null}

          <section className="mt-3 overflow-hidden rounded-xl border border-ink/8 bg-white">
            <button
              type="button"
              onClick={() => setDiagOpen((v) => !v)}
              className="flex w-full items-center justify-between gap-2 px-3.5 py-2 text-left hover:bg-surface-muted/50"
              aria-expanded={diagOpen}
            >
              <span className="text-[12px] font-semibold text-ink-secondary">
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
              <dl className="border-t border-ink/5 divide-y divide-ink/5">
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
                <Row label="Last error" value={lastError ?? "—"} mono />
              </dl>
            ) : (
              <p className="border-t border-ink/5 px-3.5 py-2 text-[11px] text-ink-muted">
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
      <dt className="w-[5.5rem] shrink-0 pt-0.5 text-[11px] font-medium text-ink-muted sm:w-24">
        {label}
      </dt>
      <dd className="min-w-0 flex-1">
        <div
          className={cn(
            "break-all text-[13px] text-ink",
            mono && "font-mono text-[12px]",
          )}
        >
          {value}
        </div>
        {hint ? (
          <p className="mt-0.5 text-[11px] leading-snug text-ink-muted">{hint}</p>
        ) : null}
      </dd>
    </div>
  );
}
