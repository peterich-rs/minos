import { open } from "@tauri-apps/plugin-dialog";
import { isTauriRuntime } from "@/shared/lib/daemon";

/**
 * Native folder picker (Tauri). Returns absolute path or null if cancelled.
 * Browser-only dev falls back to a prompt.
 */
export async function pickWorkspaceFolder(): Promise<string | null> {
  if (!isTauriRuntime()) {
    const value = window.prompt(
      "Enter workspace folder path (browser mock — use Tauri for native picker):",
      "",
    );
    const trimmed = value?.trim();
    return trimmed ? trimmed : null;
  }

  const selected = await open({
    directory: true,
    multiple: false,
    title: "Choose project workspace",
  });

  if (selected === null) return null;
  if (Array.isArray(selected)) {
    return selected[0] ?? null;
  }
  return selected;
}
