import { useEffect } from "react";
import { RefreshCw, Wifi, WifiOff } from "lucide-react";
import { useAccountStore } from "@/store/account-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";

/**
 * Persistent top banner for Account IM + Host connection.
 *
 * Product Online = Account sync (`/ws/client`). Host readiness is secondary.
 * Hidden only when Account can send/receive (accountSync online).
 * Host-only online while Account is offline still shows this banner.
 */
export function CloudConnectionBanner() {
  const session = useAccountStore((s) => s.session);
  const cloudStatus = useAccountStore((s) => s.cloudStatus);
  const accountSyncStatus = useAccountStore((s) => s.accountSyncStatus);
  const cloudError = useAccountStore((s) => s.cloudError);
  const retry = useAccountStore((s) => s.retryCloudConnection);
  const syncCloudFromHub = useAccountStore((s) => s.syncCloudFromHub);
  const hubOnline = useWorkspaceStore((s) => s.connection?.hubOnline);
  const source = useWorkspaceStore((s) => s.source);
  const refreshDaemonStatus = useWorkspaceStore((s) => s.refreshDaemonStatus);

  useEffect(() => {
    if (!session || source !== "daemon") return;
    void refreshDaemonStatus();
    const id = window.setInterval(() => {
      void refreshDaemonStatus();
    }, 5000);
    return () => window.clearInterval(id);
  }, [session, source, refreshDaemonStatus]);

  useEffect(() => {
    syncCloudFromHub(hubOnline);
  }, [hubOnline, syncCloudFromHub]);

  if (!session) return null;

  // Primary Online = Account IM. Host-only live is never enough to hide this.
  const accountOnline = accountSyncStatus === "online";
  const accountConnecting = accountSyncStatus === "connecting";
  const accountOffline = accountSyncStatus === "offline";
  const hostOffline = cloudStatus === "offline";

  // Fully healthy: Account can send/receive (Host readiness is optional secondary).
  if (accountOnline && (cloudStatus === "online" || cloudStatus === "unknown")) {
    return null;
  }
  // Account online but Host offline → soft secondary banner (bots unavailable).
  // Account unknown (boot) without host failure → stay quiet.
  if (accountSyncStatus === "unknown" && !hostOffline) {
    return null;
  }

  const connecting =
    accountConnecting ||
    (accountSyncStatus === "unknown" && cloudStatus === "connecting") ||
    (accountOnline && cloudStatus === "connecting");

  const title = connecting
    ? "Connecting…"
    : accountOffline
      ? "Messages offline"
      : hostOffline
        ? "Host offline · bots unavailable"
        : "Connecting…";

  const detail = connecting
    ? "Account sync and host runtime will work once online."
    : accountOffline
      ? "Cannot send or receive chat until Account reconnects. Local coding may still work."
      : cloudError ??
        "This Mac host is offline — bots unavailable. Humans can still chat while Account is online.";

  return (
    <div
      role="status"
      className={cn(
        "flex shrink-0 items-center justify-between gap-3 border-b px-4 py-2 text-2xs",
        connecting
          ? "border-amber-500/25 bg-amber-500/10 text-amber-950 dark:text-amber-100"
          : accountOffline
            ? "border-status-failed/30 bg-status-failed/10 text-status-failed"
            : "border-amber-500/25 bg-amber-500/10 text-amber-950 dark:text-amber-100",
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        {connecting ? (
          <Wifi className="h-3.5 w-3.5 shrink-0 animate-pulse" strokeWidth={2} />
        ) : (
          <WifiOff className="h-3.5 w-3.5 shrink-0" strokeWidth={2} />
        )}
        <div className="min-w-0">
          <p className="font-semibold">{title}</p>
          <p className="truncate opacity-90">{detail}</p>
        </div>
      </div>
      {!connecting ? (
        <button
          type="button"
          onClick={() => void retry()}
          className="inline-flex shrink-0 items-center gap-1 rounded-md bg-surface/80 px-2.5 py-1 font-semibold text-ink ring-1 ring-ink/10 transition-colors hover:bg-surface"
        >
          <RefreshCw className="h-3 w-3" strokeWidth={2} />
          Retry
        </button>
      ) : null}
    </div>
  );
}
