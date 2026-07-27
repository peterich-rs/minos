import { openUrl } from "@tauri-apps/plugin-opener";
import { useUpdaterContext } from "./hooks/UpdaterProvider";
import { Button } from "@/shared/ui/button";

/**
 * Host settings block for software updates (manual check + install).
 */
export function UpdateChecker() {
  const { status, checkForUpdate, installAndRelaunch } = useUpdaterContext();

  return (
    <section className="min-w-0" data-testid="settings-updates">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm font-medium text-ink">Software updates</p>
          <p className="mt-0.5 text-xs text-ink-muted">
            {statusLabel(status)}
          </p>
          {status.state === "error" ? (
            <p className="mt-1 text-xs text-rose-600" role="alert">
              {status.message}
            </p>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {status.state === "ready" ? (
            <Button
              size="sm"
              onClick={() => {
                void installAndRelaunch();
              }}
            >
              Install & relaunch
            </Button>
          ) : null}
          {status.state === "manual-required" ? (
            <Button
              size="sm"
              onClick={() => {
                void openUrl(status.releaseUrl);
              }}
            >
              Download update
            </Button>
          ) : null}
          {status.state !== "checking" &&
          status.state !== "downloading" &&
          status.state !== "installing" ? (
            <Button
              size="sm"
              variant={status.state === "ready" ? "outline" : "default"}
              onClick={() => {
                void checkForUpdate();
              }}
            >
              {status.state === "idle" || status.state === "up-to-date"
                ? "Check for updates"
                : "Check again"}
            </Button>
          ) : null}
        </div>
      </div>
    </section>
  );
}

function statusLabel(
  status: ReturnType<typeof useUpdaterContext>["status"],
): string {
  switch (status.state) {
    case "idle":
      return "Check whether a new desktop build is available.";
    case "checking":
      return "Checking for updates…";
    case "up-to-date":
      return "You're on the latest version.";
    case "unavailable":
      return "Automatic updates aren't available on this build (dev or unsigned). Download releases manually when needed.";
    case "available":
      return `Update v${status.version} found — preparing download…`;
    case "downloading":
      return "Downloading update…";
    case "installing":
      return "Installing update and restarting…";
    case "ready":
      return "Update downloaded. Install will stop local agents, then relaunch.";
    case "manual-required":
      return `Update v${status.version} available — this package type needs a manual download.`;
    case "error":
      return "Update check failed.";
  }
}
