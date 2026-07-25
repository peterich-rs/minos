/** @type {import('tailwindcss').Config} */
/**
 * Theme maps to CSS variables in src/index.css (Wave 1 Phase 4).
 * Use rgb(... / <alpha-value>) so utilities like bg-ink/5 keep working.
 */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  // ThemeProvider toggles `dark` on <html> when Shiki-derived luminance is dark.
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        canvas: {
          DEFAULT: "rgb(var(--color-canvas) / <alpha-value>)",
          soft: "rgb(var(--color-canvas-soft) / <alpha-value>)",
        },
        surface: {
          DEFAULT: "rgb(var(--color-surface) / <alpha-value>)",
          raised: "rgb(var(--color-surface-raised) / <alpha-value>)",
          muted: "rgb(var(--color-surface-muted) / <alpha-value>)",
          hover: "rgb(var(--color-surface-hover) / <alpha-value>)",
        },
        ink: {
          DEFAULT: "rgb(var(--color-ink) / <alpha-value>)",
          secondary: "rgb(var(--color-ink-secondary) / <alpha-value>)",
          muted: "rgb(var(--color-ink-muted) / <alpha-value>)",
          faint: "rgb(var(--color-ink-faint) / <alpha-value>)",
        },
        accent: {
          DEFAULT: "rgb(var(--color-accent) / <alpha-value>)",
          strong: "rgb(var(--color-accent-strong) / <alpha-value>)",
          soft: "rgb(var(--color-accent-soft) / <alpha-value>)",
        },
        bubble: {
          out: "rgb(var(--color-bubble-out) / <alpha-value>)",
          in: "rgb(var(--color-bubble-in) / <alpha-value>)",
        },
        status: {
          idle: "rgb(var(--color-status-idle) / <alpha-value>)",
          running: "rgb(var(--color-status-running) / <alpha-value>)",
          approval: "rgb(var(--color-status-approval) / <alpha-value>)",
          suspended: "rgb(var(--color-status-suspended) / <alpha-value>)",
          failed: "rgb(var(--color-status-failed) / <alpha-value>)",
          done: "rgb(var(--color-status-done) / <alpha-value>)",
        },
      },
      fontFamily: {
        sans: [
          "Inter",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "Segoe UI",
          "sans-serif",
        ],
      },
      // Zoom-safe meta-text tokens (timestamps, badges, kbd hints).
      // Scale with root font-size so Cmd± text zoom keeps hierarchy.
      fontSize: {
        "2xs": ["0.6875rem", { lineHeight: "1rem" }], // 11px @ 16
        "3xs": ["0.625rem", { lineHeight: "0.875rem" }], // 10px @ 16
      },
      boxShadow: {
        shell: "var(--shadow-shell)",
        panel: "var(--shadow-panel)",
      },
      borderRadius: {
        shell: "var(--radius-shell)",
        panel: "var(--radius-panel)",
        code: "var(--radius-code)",
      },
      transitionDuration: {
        150: "var(--duration-fast)",
        200: "var(--duration-normal)",
      },
      transitionTimingFunction: {
        out: "var(--ease-out)",
      },
      keyframes: {
        "message-in": {
          from: { opacity: "0", transform: "translateY(4px)" },
          to: { opacity: "1", transform: "translateY(0)" },
        },
      },
      animation: {
        "message-in": "message-in var(--duration-fast) ease-out both",
      },
    },
  },
  plugins: [require("tailwindcss-animate")],
};
