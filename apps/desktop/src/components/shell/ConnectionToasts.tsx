import { useEffect, useRef } from "react";
import { useWorkspaceStore } from "@/store/workspace-store";
import { toast } from "@/lib/toast";

/**
 * Surface daemon connection / boot failures as toasts (once per transition).
 * Mount under AppShell after boot completes.
 */
export function ConnectionToasts() {
  const connection = useWorkspaceStore((s) => s.connection);
  const source = useWorkspaceStore((s) => s.source);
  const error = useWorkspaceStore((s) => s.error);
  const booting = useWorkspaceStore((s) => s.booting);
  const prevConnected = useRef<boolean | null>(null);
  const lastError = useRef<string | null>(null);

  useEffect(() => {
    if (booting || source !== "daemon") return;
    const connected = connection?.connected === true;
    const prev = prevConnected.current;

    if (prev === null) {
      prevConnected.current = connected;
      return;
    }

    if (prev && !connected) {
      toast.error(
        "Daemon disconnected",
        connection?.error ?? error ?? "Local runtime is unavailable",
      );
    } else if (!prev && connected) {
      toast.success(
        "Daemon connected",
        connection?.managed ? "Managed process ready" : "Ready",
      );
    }
    prevConnected.current = connected;
  }, [booting, source, connection?.connected, connection?.error, connection?.managed, error]);

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
