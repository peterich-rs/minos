/**
 * Recover desktop runtime after a failed prepare/install/relaunch path.
 *
 * `prepare_for_app_update` stops the managed daemon; if the binary swap does
 * not complete, we must bring RPC back without requiring a full app restart.
 */
import { invoke } from "@tauri-apps/api/core";
import type { DaemonConnection } from "@/shared/lib/daemon";
import { isTauriRuntime } from "@/shared/lib/runtime";
import { useWorkspaceStore } from "@/store/workspace-store";
import type { RestoreOutcome } from "./update-failure-message";

export type { RestoreOutcome };
export { formatUpdateFailureMessage } from "./update-failure-message";

export type RestoreAfterFailedUpdateResult = RestoreOutcome & {
  connection: DaemonConnection | null;
};

/**
 * Reset the prepare guard, restart managed daemon, and refresh workspace
 * connection so the shell is usable again.
 */
export async function restoreRuntimeAfterFailedUpdate(): Promise<RestoreAfterFailedUpdateResult> {
  if (!isTauriRuntime()) {
    return { restored: true, connection: null };
  }

  try {
    const connection = await invoke<DaemonConnection>("restore_after_failed_update");
    useWorkspaceStore.setState({
      connection,
      error: connection.connected ? null : (connection.error ?? "Daemon unavailable"),
      // Pumps re-arm on connect; push-status events will flip livePush true.
      livePush: false,
    });
    return {
      restored: connection.connected,
      connection,
      error: connection.connected
        ? undefined
        : (connection.error ?? "Daemon restore returned disconnected"),
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    useWorkspaceStore.setState({
      connection: {
        connected: false,
        endpoint: null,
        error: message,
        source: "error",
        managed: false,
      },
      error: message,
      livePush: false,
    });
    return { restored: false, connection: null, error: message };
  }
}
