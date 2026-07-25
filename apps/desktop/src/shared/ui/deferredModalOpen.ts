import * as React from "react";

/** Match MODAL_CONTENT_MOTION_CLASS closed duration. */
export const MODAL_EXIT_ANIMATION_MS = 150;

/**
 * Open the next modal after the previous one's exit animation, or on rAF.
 * Prevents stacked dialogs from fighting enter/exit transitions.
 */
export function useDeferredModalOpen() {
  const frameRef = React.useRef<number | null>(null);
  const timeoutRef = React.useRef<number | null>(null);

  const cancelDeferredModalOpen = React.useCallback(() => {
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  }, []);

  const openNextFrame = React.useCallback(
    (open: () => void) => {
      cancelDeferredModalOpen();
      frameRef.current = window.requestAnimationFrame(() => {
        frameRef.current = null;
        open();
      });
    },
    [cancelDeferredModalOpen],
  );

  const openAfterExit = React.useCallback(
    (open: () => void) => {
      cancelDeferredModalOpen();
      timeoutRef.current = window.setTimeout(() => {
        timeoutRef.current = null;
        openNextFrame(open);
      }, MODAL_EXIT_ANIMATION_MS);
    },
    [cancelDeferredModalOpen, openNextFrame],
  );

  React.useEffect(() => cancelDeferredModalOpen, [cancelDeferredModalOpen]);

  return {
    cancelDeferredModalOpen,
    openAfterExit,
    openNextFrame,
  };
}
