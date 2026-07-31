import {
  type ReactNode,
  type Ref,
  type TextareaHTMLAttributes,
} from "react";
import {
  AtSign,
  Bold,
  Paperclip,
  Send,
  Smile,
} from "lucide-react";

import { cn } from "@/shared/lib/utils";

type TextareaProps = TextareaHTMLAttributes<HTMLTextAreaElement> & {
  ref?: Ref<HTMLTextAreaElement>;
};

/**
 * Presentational composer dock shared by Desktop chat + Web cloud mock.
 * Logic (mentions, send, emoji picker) stays in the host feature.
 */
export function ComposerChrome({
  textareaProps,
  toolbarStart,
  sendLabel = "Send",
  sendDisabled = true,
  onSend,
  hint,
  className,
  footerClassName,
}: {
  textareaProps?: TextareaProps;
  /** Live toolbar (Desktop). When omitted, shows disabled Buzz-style tools. */
  toolbarStart?: ReactNode;
  sendLabel?: string;
  sendDisabled?: boolean;
  onSend?: () => void;
  hint?: ReactNode;
  className?: string;
  footerClassName?: string;
}) {
  const {
    className: taClassName,
    rows = 3,
    placeholder = "Message… type @ to mention an agent (e.g. @grok hello)",
    ref: taRef,
    ...restTa
  } = textareaProps ?? {};

  return (
    <div
      className={cn(
        "relative shrink-0 border-t border-ink/6 bg-surface px-4 py-4 sm:px-5",
        className,
      )}
    >
      <div
        className={cn(
          "rounded-2xl border border-ink/10 bg-surface shadow-sm ring-1 ring-ink/5",
          "focus-within:ring-2 focus-within:ring-primary/30",
        )}
      >
        <textarea
          ref={taRef}
          rows={rows}
          placeholder={placeholder}
          className={cn(
            "w-full resize-none rounded-t-2xl bg-transparent px-4 pt-3 text-sm text-ink outline-none placeholder:text-ink-muted",
            "disabled:cursor-not-allowed disabled:opacity-70",
            taClassName,
          )}
          {...restTa}
        />
        <div className="flex items-center justify-between border-t border-ink/5 px-3 py-2.5">
          <div className="flex items-center gap-0.5 text-ink-muted">
            {toolbarStart ?? (
              <>
                <ComposerToolBtn title="@ mention" disabled>
                  <AtSign className="h-3.5 w-3.5" />
                </ComposerToolBtn>
                <ComposerToolBtn title="Bold" disabled>
                  <Bold className="h-3.5 w-3.5" />
                </ComposerToolBtn>
                <ComposerToolBtn title="Emoji" disabled>
                  <Smile className="h-3.5 w-3.5" />
                </ComposerToolBtn>
                <ComposerToolBtn title="Attach" disabled>
                  <Paperclip className="h-3.5 w-3.5" />
                </ComposerToolBtn>
              </>
            )}
          </div>
          <button
            type="button"
            disabled={sendDisabled}
            onClick={onSend}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-xl bg-primary px-3.5 py-2 text-xs font-semibold text-white shadow-sm",
              "hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40",
            )}
          >
            {sendLabel}
            <Send className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
      {hint ? (
        <p
          className={cn(
            "mt-2 px-1 text-2xs text-ink-muted",
            footerClassName,
          )}
        >
          {hint}
        </p>
      ) : null}
    </div>
  );
}

export function ComposerToolBtn({
  children,
  onClick,
  disabled,
  title,
  className,
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
  className?: string;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex h-7 w-7 items-center justify-center rounded-md text-ink-muted",
        "hover:bg-surface-hover hover:text-ink",
        "disabled:pointer-events-none disabled:opacity-45",
        className,
      )}
    >
      {children}
    </button>
  );
}
