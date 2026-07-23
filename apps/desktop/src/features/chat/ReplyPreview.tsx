import type { TimelineMessage } from "@/shared/lib/mock-data";
import { cn } from "@/shared/lib/utils";
import { replyAuthorLabel, replyPreviewBody } from "./lib/format";

export function ReplyPreview({
  replyToMessageId,
  replyParent,
  isUser,
}: {
  replyToMessageId: string;
  replyParent?: TimelineMessage;
  isUser: boolean;
}) {
  return (
    <div
      className={cn(
        "rounded-lg border-l-2 px-2.5 py-1.5 text-[11.5px] leading-snug",
        isUser
          ? "border-white/40 bg-black/15 text-white/85"
          : "border-ink/20 bg-surface-muted/80 text-ink-secondary",
      )}
    >
      <div className={cn("font-medium", isUser ? "text-white" : "text-ink")}>
        ↳ {replyParent ? replyAuthorLabel(replyParent) : "Reply"}
      </div>
      <div className="mt-0.5 line-clamp-2 opacity-90">
        {replyParent
          ? replyPreviewBody(replyParent.body)
          : `(reply unavailable · ${replyToMessageId})`}
      </div>
    </div>
  );
}
