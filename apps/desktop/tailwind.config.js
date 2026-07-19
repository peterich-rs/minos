/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: {
          DEFAULT: "#ebe4d8",
          soft: "#f4efe6",
        },
        surface: {
          DEFAULT: "#fbf8f3",
          raised: "#ffffff",
          muted: "#f3eee5",
          hover: "#efe8dc",
        },
        ink: {
          DEFAULT: "#1c1917",
          secondary: "#57534e",
          muted: "#a8a29e",
          faint: "#d6d3d1",
        },
        accent: {
          DEFAULT: "#f472b6",
          strong: "#ec4899",
          soft: "#fce7f3",
        },
        bubble: {
          out: "#1c1917",
          in: "#ffffff",
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
      boxShadow: {
        shell: "0 24px 80px rgba(28, 25, 23, 0.12), 0 2px 8px rgba(28, 25, 23, 0.04)",
        panel: "0 1px 0 rgba(28, 25, 23, 0.04)",
      },
      borderRadius: {
        shell: "18px",
        panel: "14px",
      },
    },
  },
  plugins: [],
};
