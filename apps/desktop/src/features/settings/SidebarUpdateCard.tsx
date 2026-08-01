import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CircleArrowUp, ExternalLink } from "lucide-react";
import { useUpdaterContext } from "./hooks/UpdaterProvider";
import { shouldShowSidebarUpdateCard } from "./sidebarUpdateCardVisibility";
import { SidebarActionCard } from "@/shared/ui/sidebar-action-card";
import { SidebarGlassCard } from "@/shared/ui/SidebarGlassCard";

type Props = {
  onDismiss: () => void;
};

export function SidebarUpdateCard({ onDismiss }: Props) {
  const { status, installAndRelaunch } = useUpdaterContext();
  const [pending, setPending] = useState(false);

  if (!shouldShowSidebarUpdateCard(status)) {
    return null;
  }

  const installing =
    pending || status.state === "installing" || status.state === "downloading";

  if (status.state === "manual-required") {
    return (
      <SidebarGlassCard tone="success">
        <SidebarActionCard
          testId="sidebar-update-card"
          tone="success"
          icon={<ExternalLink className="h-4 w-4 text-status-done" />}
          title={`Update v${status.version}`}
          description="In-app update not supported on this package. Download from GitHub (AppImage for auto-updates on Linux)."
          actionLabel="Download"
          onAction={() => {
            void openUrl(status.releaseUrl);
          }}
          onDismiss={onDismiss}
          dismissLabel="Dismiss update notification"
          className="border-0 bg-transparent shadow-none"
        />
      </SidebarGlassCard>
    );
  }

  const version = status.state === "available" ? status.version : undefined;

  return (
    <SidebarGlassCard tone="success">
      <SidebarActionCard
        testId="sidebar-update-card"
        tone="success"
        icon={<CircleArrowUp className="h-4 w-4 text-status-done" />}
        title={version ? `Update v${version}` : "Update available"}
        description={
          installing
            ? status.state === "downloading"
              ? "Downloading…"
              : "Installing…"
            : "Ready to install and relaunch"
        }
        actionLabel={installing ? "Working…" : "Update now"}
        actionDisabled={installing}
        onAction={() => {
          if (installing) return;
          setPending(true);
          void installAndRelaunch()
            .catch((error) => {
              console.error("[SidebarUpdateCard] update failed:", error);
            })
            .finally(() => setPending(false));
        }}
        onDismiss={installing ? undefined : onDismiss}
        dismissLabel="Dismiss update notification"
        className="border-0 bg-transparent shadow-none"
      />
    </SidebarGlassCard>
  );
}
