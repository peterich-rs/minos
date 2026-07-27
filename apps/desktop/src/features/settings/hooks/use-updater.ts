import { useState, useRef, useCallback, useEffect } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { invoke } from "@tauri-apps/api/core";
import { isTauriRuntime } from "@/shared/lib/runtime";
import {
  formatUpdateFailureMessage,
  restoreRuntimeAfterFailedUpdate,
} from "./restore-after-failed-update";

export type UpdateStatus =
  | { state: "idle" }
  | { state: "checking" }
  | { state: "up-to-date" }
  | { state: "unavailable" }
  | { state: "available"; version: string }
  | { state: "downloading" }
  | { state: "installing" }
  | { state: "ready" }
  | { state: "error"; message: string }
  | {
      state: "manual-required";
      version: string;
      /** GitHub releases page for the update. */
      releaseUrl: string;
    };

const BACKGROUND_UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const BACKGROUND_BLOCKED_STATES = new Set<UpdateStatus["state"]>([
  "checking",
  "available",
  "downloading",
  "installing",
  "ready",
  "manual-required",
]);

/** Override via VITE_MINOS_RELEASES_URL when the repo path differs. */
const GITHUB_RELEASES_URL =
  (import.meta.env.VITE_MINOS_RELEASES_URL as string | undefined) ||
  "https://github.com/peterich-rs/minos/releases/latest";

function toErrorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function isUpdaterUnavailable(message: string): boolean {
  return (
    message.includes("plugin updater not found") ||
    message.includes("not initialized") ||
    message.includes("Command updater") ||
    message.includes("not allowed")
  );
}

function canRunBackgroundCheck(status: UpdateStatus): boolean {
  return !BACKGROUND_BLOCKED_STATES.has(status.state);
}

function initialUpdateStatus(): UpdateStatus {
  return { state: "idle" };
}

async function isAutoUpdateSupported(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  try {
    return await invoke<boolean>("is_auto_update_supported");
  } catch {
    return false;
  }
}

async function isUpdaterPluginEnabled(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  try {
    return await invoke<boolean>("is_updater_plugin_enabled");
  } catch {
    return false;
  }
}

/** Phase C: stop managed daemon / agents before install + relaunch. */
async function prepareForAppUpdate(): Promise<void> {
  if (!isTauriRuntime()) return;
  await invoke("prepare_for_app_update");
}

export function useUpdater() {
  const [status, setStatusState] = useState<UpdateStatus>(initialUpdateStatus);
  const statusRef = useRef<UpdateStatus>(initialUpdateStatus());
  const updateRef = useRef<Update | null>(null);
  const checkInFlightRef = useRef(false);
  const downloadInFlightRef = useRef(false);
  const installInFlightRef = useRef(false);
  const manualResultRequestedRef = useRef(false);

  const setStatus = useCallback((nextStatus: UpdateStatus) => {
    statusRef.current = nextStatus;
    setStatusState(nextStatus);
  }, []);

  const closeUpdate = useCallback(async () => {
    if (downloadInFlightRef.current || installInFlightRef.current) {
      return;
    }
    const current = updateRef.current;
    if (current) {
      updateRef.current = null;
      await current.close();
    }
  }, []);

  const downloadUpdate = useCallback(async () => {
    if (downloadInFlightRef.current) {
      return;
    }

    downloadInFlightRef.current = true;
    try {
      const update = updateRef.current;
      if (!update) {
        return;
      }

      setStatus({ state: "downloading" });
      await update.download();
      setStatus({ state: "ready" });
    } catch (err) {
      setStatus({ state: "error", message: toErrorMessage(err) });
    } finally {
      downloadInFlightRef.current = false;
    }
  }, [setStatus]);

  const installAndRelaunch = useCallback(async () => {
    if (installInFlightRef.current) {
      return;
    }

    const update = updateRef.current;
    if (!update) {
      return;
    }

    installInFlightRef.current = true;
    /** True once prepare may have stopped the managed daemon. */
    let teardownStarted = false;
    try {
      setStatus({ state: "installing" });
      // Tear down managed daemon / agents before binary swap. Failure blocks
      // install; any failure after this point must restore runtime.
      teardownStarted = true;
      await prepareForAppUpdate();
      await update.install();
      updateRef.current = null;
      await relaunch();
    } catch (err) {
      const installError = toErrorMessage(err);
      if (teardownStarted) {
        // prepare / install / relaunch failed after we may have stopped the
        // daemon — bring local RPC back so the shell is not left dead.
        const restore = await restoreRuntimeAfterFailedUpdate();
        setStatus({
          state: "error",
          message: formatUpdateFailureMessage(installError, restore),
        });
      } else {
        setStatus({ state: "error", message: installError });
      }
    } finally {
      installInFlightRef.current = false;
    }
  }, [setStatus]);

  const runUpdateCheck = useCallback(
    async ({ background }: { background: boolean }) => {
      if (!isTauriRuntime()) {
        if (!background) {
          setStatus({ state: "unavailable" });
        }
        return;
      }

      if (checkInFlightRef.current) {
        if (!background) {
          manualResultRequestedRef.current = true;
          setStatus({ state: "checking" });
        }
        return;
      }

      if (background && !canRunBackgroundCheck(statusRef.current)) {
        return;
      }

      checkInFlightRef.current = true;
      manualResultRequestedRef.current = false;

      try {
        await closeUpdate();

        if (!background) {
          setStatus({ state: "checking" });
        }

        const pluginOn = await isUpdaterPluginEnabled();
        if (!pluginOn) {
          if (!background || manualResultRequestedRef.current) {
            setStatus({ state: "unavailable" });
          }
          return;
        }

        const update = await check({
          headers: { "Cache-Control": "no-cache" },
        });
        const shouldShowQuietResult =
          !background || manualResultRequestedRef.current;

        if (update) {
          // Check support BEFORE exposing any actionable state — on a Linux
          // .deb, the window between "available" and "manual-required" would
          // let a click reach an un-updatable install.
          const autoUpdateOk = await isAutoUpdateSupported();
          updateRef.current = update;
          if (autoUpdateOk) {
            setStatus({ state: "available", version: update.version });
            void downloadUpdate();
          } else {
            updateRef.current = null;
            setStatus({
              state: "manual-required",
              version: update.version,
              releaseUrl: GITHUB_RELEASES_URL,
            });
          }
        } else if (shouldShowQuietResult) {
          setStatus({ state: "up-to-date" });
        }
      } catch (err) {
        const message = toErrorMessage(err);
        const shouldShowQuietResult =
          !background || manualResultRequestedRef.current;

        if (isUpdaterUnavailable(message)) {
          console.warn(`updater unavailable: ${message}`);
          if (shouldShowQuietResult) {
            setStatus({ state: "unavailable" });
          }
          return;
        }

        if (shouldShowQuietResult) {
          setStatus({ state: "error", message });
        }
      } finally {
        manualResultRequestedRef.current = false;
        checkInFlightRef.current = false;
      }
    },
    [closeUpdate, downloadUpdate, setStatus],
  );

  const checkForUpdate = useCallback(async () => {
    await runUpdateCheck({ background: false });
  }, [runUpdateCheck]);

  const checkForUpdateInBackground = useCallback(async () => {
    await runUpdateCheck({ background: true });
  }, [runUpdateCheck]);

  useEffect(() => {
    void checkForUpdateInBackground();

    const intervalId = window.setInterval(() => {
      void checkForUpdateInBackground();
    }, BACKGROUND_UPDATE_CHECK_INTERVAL_MS);

    return () => {
      window.clearInterval(intervalId);
      void closeUpdate();
    };
  }, [checkForUpdateInBackground, closeUpdate]);

  return {
    status,
    checkForUpdate,
    installAndRelaunch,
  };
}
