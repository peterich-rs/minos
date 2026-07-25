/**
 * Shared Radix dialog / modal motion tokens.
 * Keep overlay + content enter/exit in one place so every surface matches.
 */

export const MODAL_OVERLAY_MOTION_CLASS =
  "duration-200 ease-out data-[state=closed]:duration-150 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 motion-reduce:animate-none";

export const MODAL_CONTENT_MOTION_CLASS =
  "origin-center duration-200 ease-out data-[state=closed]:duration-150 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 motion-reduce:animate-none motion-reduce:data-[state=open]:zoom-in-100 motion-reduce:data-[state=closed]:zoom-out-100";
