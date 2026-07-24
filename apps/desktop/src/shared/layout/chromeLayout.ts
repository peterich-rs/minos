/**
 * Shell geometry tokens.
 *
 * Readable text stays rem-based; chrome dimensions that align with fixed
 * native controls / panel rails use CSS variables so one place owns the scale.
 * Title bar is still system-decorated (no traffic-light inset yet) — vars are
 * ready when we switch to hidden titlebar.
 */

export const SIDEBAR_WIDTH_DEFAULT = "240px";
export const TOP_CHROME_HEIGHT_DEFAULT = "0px";
export const AUXILIARY_PANEL_DEFAULT_WIDTH_PX = 280;
export const AUXILIARY_PANEL_MIN_WIDTH_PX = 220;
export const AUXILIARY_PANEL_MAX_WIDTH_PX = 420;
/** Below this viewport width, inspector becomes a floating overlay. */
export const AUXILIARY_PANEL_OVERLAY_BREAKPOINT_PX = 1100;

export const chromeCssVars = {
  sidebarWidth: "--minos-sidebar-width",
  topChromeHeight: "--minos-top-chrome-height",
  auxiliaryPanelWidth: "--minos-auxiliary-panel-width",
} as const;

export const chromeCssVarDefaults = {
  [chromeCssVars.sidebarWidth]: SIDEBAR_WIDTH_DEFAULT,
  [chromeCssVars.topChromeHeight]: TOP_CHROME_HEIGHT_DEFAULT,
  [chromeCssVars.auxiliaryPanelWidth]: `${AUXILIARY_PANEL_DEFAULT_WIDTH_PX}px`,
} as const;

/** Tailwind-friendly class fragments for chrome-aware layout. */
export const shellChrome = {
  sidebarWidth: "w-[var(--minos-sidebar-width,240px)]",
  auxiliaryWidth:
    "w-[var(--minos-auxiliary-panel-width,280px)] min-w-[220px] max-w-[min(420px,90vw)]",
} as const;
