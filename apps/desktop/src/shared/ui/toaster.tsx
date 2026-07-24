import { Toaster as Sonner } from "sonner";

/** App-wide toast host (sonner). Mount once under AppShell. */
export function Toaster() {
  return (
    <Sonner
      position="bottom-right"
      closeButton
      theme="light"
      toastOptions={{
        classNames: {
          toast:
            "group border border-ink/10 bg-surface text-ink shadow-lg rounded-xl",
          title: "text-sm font-semibold text-ink",
          description: "text-xs text-ink-muted",
          actionButton: "bg-ink text-white",
          cancelButton: "bg-surface-muted text-ink-secondary",
          error: "border-rose-200/80",
          success: "border-emerald-200/80",
        },
      }}
    />
  );
}
