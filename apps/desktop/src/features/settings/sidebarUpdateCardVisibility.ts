import type { UpdateStatus } from "./hooks/use-updater";

/** Whether the sidebar should show a compact update nudge. */
export function shouldShowSidebarUpdateCard(status: {
  state: UpdateStatus["state"];
}): boolean {
  return (
    status.state === "ready" ||
    status.state === "installing" ||
    status.state === "manual-required" ||
    status.state === "downloading" ||
    status.state === "available"
  );
}
