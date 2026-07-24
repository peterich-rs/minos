import { invoke } from "@tauri-apps/api/core";
import { isTauriRuntime } from "@/shared/lib/runtime";
import { DaemonInvokeError } from "./daemon-invoke-error";

export { DaemonInvokeError } from "./daemon-invoke-error";

/**
 * Single entry point for daemon Tauri `invoke` calls.
 * Wraps failures as `DaemonInvokeError` with the command name attached.
 */
export async function invokeDaemon<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!isTauriRuntime()) {
    throw new DaemonInvokeError("not running in Tauri", command);
  }
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new DaemonInvokeError(message, command, error);
  }
}
