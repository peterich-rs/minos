import { useEffect, useRef, useState } from "react";
import { Unplug } from "lucide-react";
import { useWorkspaceStore } from "@/store/workspace-store";
import { useUiStore } from "@/store/ui-store";
import { planConnectionCardVisibility } from "@/shared/lib/connection-card-policy";
import { DISCONNECT_TOAST_DEBOUNCE_MS } from "@/shared/lib/connection-toast-policy";
import { SidebarActionCard } from "@/shared/ui/sidebar-action-card";

/**
 * Persistent sidebar card when the local daemon is down.
 * Debounced with the same window as disconnect toasts so brief flaps stay quiet.
 * Dismiss lasts for the current disconnect episode (cleared on reconnect).
 */
export function SidebarConnectionCard() {
  const connection = useWorkspaceStore((s) => s.connection);
  const source = useWorkspaceStore((s) => s.source);
  const booting = useWorkspaceStore((s) => s.booting);
  const error = useWorkspaceStore((s) => s.error);
  const bootstrap = useWorkspaceStore((s) => s.bootstrap);
  const setPrimaryNav = useUiStore((s) => s.setPrimaryNav);

  const connected = source === "daemon" && connection?.connected === true;
  const [dismissed, setDismissed] = useState(false);
  const [stableDisconnected, setStableDisconnected] = useState(false);
  const disconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Debounce "show card" the same way toasts debounce disconnect.
  useEffect(() => {
    if (booting || source !== "daemon") {
      setStableDisconnected(false);
      setDismissed(false);
      return;
    }

    if (connected) {
      if (disconnectTimer.current) {
        clearTimeout(disconnectTimer.current);
        disconnectTimer.current = null;
      }
      setStableDisconnected(false);
      setDismissed(false);
      return;
    }

    // disconnected
    if (disconnectTimer.current) clearTimeout(disconnectTimer.current);
    disconnectTimer.current = setTimeout(() => {
      setStableDisconnected(true);
      disconnectTimer.current = null;
    }, DISCONNECT_TOAST_DEBOUNCE_MS);

    return () => {
      if (disconnectTimer.current) {
        clearTimeout(disconnectTimer.current);
        disconnectTimer.current = null;
      }
    };
  }, [booting, source, connected]);

  const visibility = planConnectionCardVisibility({
    booting,
    source: source === "daemon" ? "daemon" : "mock",
    connected: !stableDisconnected,
    dismissed,
  });

  if (visibility !== "show") {
    return null;
  }

  const detail =
    connection?.error || error || "Local runtime is unavailable";

  return (
    <div className="border-t border-ink/5 px-2 py-2">
      <SidebarActionCard
        testId="sidebar-connection-card"
        role="alert"
        tone="danger"
        icon={<Unplug className="h-4 w-4 text-rose-600" />}
        title="Daemon offline"
        description={detail}
        actionLabel="Retry"
        onAction={() => {
          void bootstrap();
        }}
        secondaryLabel="Host"
        onSecondary={() => setPrimaryNav("host")}
        onDismiss={() => setDismissed(true)}
        dismissLabel="Dismiss connection warning"
      />
    </div>
  );
}
