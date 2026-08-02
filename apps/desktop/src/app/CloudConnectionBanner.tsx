import { useEffect } from "react";
import { RefreshCw, Wifi, WifiOff } from "lucide-react";
import { useAccountStore } from "@/store/account-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";

/**
 * Persistent top banner for cloud (server) connection.
 * Online: hidden. Connecting / Offline: shown with optional Retry.
 */
export function CloudConnectionBanner() {
  const session = useAccountStore((s) => s.session);
  const cloudStatus = useAccountStore((s) => s.cloudStatus);
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
  if (cloudStatus === "online" || cloudStatus === "unknown") return null;

  const connecting = cloudStatus === "connecting";

  return (
    <div
      role="status"
      className={cn(
        "flex shrink-0 items-center justify-between gap-3 border-b px-4 py-2 text-2xs",
        connecting
          ? "border-amber-500/25 bg-amber-500/10 text-amber-950 dark:text-amber-100"
          : "border-status-failed/30 bg-status-failed/10 text-status-failed",
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        {connecting ? (
          <Wifi className="h-3.5 w-3.5 shrink-0 animate-pulse" strokeWidth={2} />
        ) : (
          <WifiOff className="h-3.5 w-3.5 shrink-0" strokeWidth={2} />
        )}
        <div className="min-w-0">
          <p className="font-semibold">
            {connecting
              ? "Connecting to server…"
              : "Disconnected from server"}
          </p>
          <p className="truncate opacity-90">
            {connecting
              ? "Phone and remote control will work once online."
              : cloudError ??
                "Local coding still works. Remote / phone control is unavailable."}
          </p>
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
