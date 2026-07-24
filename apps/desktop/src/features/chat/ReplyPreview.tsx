import type { TimelineMessage } from "@/shared/lib/mock-data";
import { replyAuthorLabel, replyPreviewBody } from "./lib/format";

export function ReplyPreview({
  replyToMessageId,
  replyParent,
}: {
  replyToMessageId: string;
  replyParent?: TimelineMessage;
}) {
  return (
    <div className="mb-1 rounded-lg border-l-2 border-ink/20 bg-surface-muted/80 px-2.5 py-1.5 text-[11.5px] leading-snug text-ink-secondary">
      <div className="font-medium text-ink">
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
