import { ShieldAlert } from "lucide-react";
import type { TranscriptItem } from "@/shared/lib/daemon";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { IncrementalText } from "@/shared/ui/IncrementalText";
import type { UserAction } from "../lib/user-action";

export function ApprovalModal({
  item,
  isPlan,
  open,
  approving,
  onClose,
  onUserAction,
}: {
  item: TranscriptItem;
  isPlan: boolean;
  open: boolean;
  approving?: boolean;
  onClose: () => void;
  onUserAction?: (action: UserAction) => void | Promise<void>;
}) {
  const detail = item.detail?.trim() ? item.detail : null;
  // Plans can be multi‑10KB markdown; windowed display avoids one-shot paint.
  // Other approval details stay small (assembler already caps ~2KB).
  const useIncremental = isPlan && Boolean(detail) && detail!.length > 4_000;
  const options = item.options ?? [];
  const isQuestion =
    item.kind === "question" ||
    item.approvalMethod === "opencode/question" ||
    item.approvalMethod === "x.ai/ask_user_question";

  const runAction = async (action: UserAction) => {
    try {
      // Success toast is owned by the transcript onUserAction callback.
      await onUserAction?.(action);
      onClose();
    } catch (e) {
      // onUserAction may already toast; still close only on success.
      if (e) {
        /* keep open for retry */
      }
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent
        hideClose
        className="flex max-h-[min(85vh,720px)] w-full max-w-2xl flex-col gap-0 overflow-hidden p-0 sm:rounded-2xl"
        aria-describedby={undefined}
      >
        <DialogHeader className="shrink-0 space-y-1.5 pr-12">
          <DialogTitle className="flex items-center gap-2">
            <ShieldAlert className="h-4 w-4 shrink-0 text-rose-600" />
            {item.title ?? "Approval required"}
          </DialogTitle>
          <DialogDescription className="whitespace-pre-wrap text-left">
            {item.text}
          </DialogDescription>
          <button
            type="button"
            onClick={onClose}
            className="absolute right-4 top-4 rounded-lg px-2 py-1 text-xs font-medium text-ink-muted transition-colors duration-150 hover:bg-surface-muted hover:text-ink"
          >
            Close
          </button>
        </DialogHeader>
        {detail ? (
          useIncremental ? (
            <IncrementalText text={detail} className="min-h-0 px-5 py-4" />
          ) : (
            <div className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-5 py-4">
              <pre className="whitespace-pre-wrap font-mono text-xs leading-relaxed text-ink-secondary">
                {detail}
              </pre>
            </div>
          )
        ) : isQuestion && options.length > 0 ? (
          <div className="scrollbar-thin min-h-0 flex-1 space-y-2 overflow-y-auto px-5 py-4">
            {options.map((opt) => (
              <button
                key={opt.label}
                type="button"
                disabled={approving}
                onClick={() => {
                  void runAction({ type: "decision", decision: opt.label });
                }}
                className="flex w-full flex-col rounded-xl border border-ink/10 bg-white px-3.5 py-2.5 text-left transition-colors duration-150 hover:border-ink/25 hover:bg-surface-muted/60 disabled:opacity-50"
              >
                <span className="text-sm font-semibold text-ink">
                  {opt.label}
                </span>
                {opt.description ? (
                  <span className="mt-0.5 text-xs text-ink-muted">
                    {opt.description}
                  </span>
                ) : null}
              </button>
            ))}
          </div>
        ) : (
          <div className="min-h-0 flex-1 px-5 py-4">
            <p className="text-sm text-ink-muted">
              {isQuestion
                ? "Pick an option above or cancel."
                : "No additional detail."}
            </p>
          </div>
        )}
        {onUserAction ? (
          <DialogFooter className="shrink-0">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={approving}
              onClick={() => {
                void runAction(
                  isQuestion
                    ? { type: "cancel" }
                    : {
                        type: "decision",
                        decision: isPlan ? "abandon" : "deny",
                      },
                );
              }}
            >
              {isPlan ? "Abandon" : isQuestion ? "Cancel" : "Deny"}
            </Button>
            {isPlan ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={approving}
                onClick={() => {
                  void runAction({ type: "decision", decision: "revise" });
                }}
              >
                Request changes
              </Button>
            ) : null}
            {!isQuestion ? (
              <Button
                type="button"
                size="sm"
                disabled={approving}
                onClick={() => {
                  void runAction({ type: "decision", decision: "approve" });
                }}
              >
                {isPlan ? "Approve plan" : "Allow"}
              </Button>
            ) : null}
          </DialogFooter>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
