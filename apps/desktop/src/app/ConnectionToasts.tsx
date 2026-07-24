import { useEffect, useRef } from "react";
import { useWorkspaceStore } from "@/store/workspace-store";
import { toast } from "@/shared/lib/toast";
import {
  DISCONNECT_TOAST_DEBOUNCE_MS,
  planConnectionToast,
} from "@/shared/lib/connection-toast-policy";

/**
 * Surface daemon connection / boot failures as toasts (once per transition).
 * Disconnect is debounced so brief flaps do not flash error → success.
 * Mount under AppShell after boot completes.
 */
export function ConnectionToasts() {
  const connection = useWorkspaceStore((s) => s.connection);
  const source = useWorkspaceStore((s) => s.source);
  const error = useWorkspaceStore((s) => s.error);
  const booting = useWorkspaceStore((s) => s.booting);
  /** Last **committed** connected flag used for toasts (after debounce). */
  const prevConnected = useRef<boolean | null>(null);
  const lastError = useRef<string | null>(null);
  const disconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingDisconnect = useRef(false);

  useEffect(() => {
    return () => {
      if (disconnectTimer.current) {
        clearTimeout(disconnectTimer.current);
        disconnectTimer.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (booting || source !== "daemon") return;
    const connected = connection?.connected === true;
    const prev = prevConnected.current;
    const disconnectMessage =
      connection?.error ?? error ?? "Local runtime is unavailable";
    const connectedDetail = connection?.managed
      ? "Managed process ready"
      : "Ready";

    if (prev === null) {
      prevConnected.current = connected;
      return;
    }

    const action = planConnectionToast({
      prev,
      connected,
      pendingDisconnect: pendingDisconnect.current,
      disconnectMessage,
      connectedDetail,
    });

    switch (action.type) {
      case "none":
        break;
      case "schedule_disconnect": {
        pendingDisconnect.current = true;
        if (disconnectTimer.current) clearTimeout(disconnectTimer.current);
        const message = action.message;
        disconnectTimer.current = setTimeout(() => {
          toast.error("Daemon disconnected", message);
          prevConnected.current = false;
          pendingDisconnect.current = false;
          disconnectTimer.current = null;
        }, DISCONNECT_TOAST_DEBOUNCE_MS);
        break;
      }
      case "cancel_pending": {
        if (disconnectTimer.current) {
          clearTimeout(disconnectTimer.current);
          disconnectTimer.current = null;
        }
        pendingDisconnect.current = false;
        prevConnected.current = true;
        break;
      }
      case "toast_connected": {
        toast.success("Daemon connected", action.detail);
        prevConnected.current = true;
        break;
      }
      case "commit_disconnected": {
        prevConnected.current = false;
        break;
      }
    }
  }, [
    booting,
    source,
    connection?.connected,
    connection?.error,
    connection?.managed,
    error,
  ]);

  useEffect(() => {
    if (booting || !error) return;
    if (lastError.current === error) return;
    lastError.current = error;
    // Avoid double-toasting with disconnect path when connection already false.
    if (connection?.connected === false) return;
    toast.error("Workspace error", error);
  }, [booting, error, connection?.connected]);

  return null;
}
