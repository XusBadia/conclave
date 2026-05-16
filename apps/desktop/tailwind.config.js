/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        bg: {
          DEFAULT: "#0b0f14",
          subtle: "#0f1419",
          elevated: "#131820",
        },
        surface: {
          DEFAULT: "#131820",
          hover: "#1a212c",
          active: "#222a37",
        },
        border: {
          DEFAULT: "#1f2630",
          subtle: "#172029",
          strong: "#2a3340",
        },
        ink: {
          DEFAULT: "#f5f6f8",
          dim: "#cbd5e1",
          subtle: "#94a3b8",
          faint: "#64748b",
        },
        accent: {
          DEFAULT: "#7dd3fc",
          strong: "#38bdf8",
          muted: "#0c4a6e",
        },
        ok: "#34d399",
        warn: "#fbbf24",
        danger: "#f87171",
      },
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          "SF Pro Display",
          "SF Pro Text",
          "Inter",
          "system-ui",
          "sans-serif",
        ],
        mono: [
          "SF Mono",
          "JetBrains Mono",
          "Menlo",
          "Monaco",
          "Consolas",
          "monospace",
        ],
      },
      boxShadow: {
        soft: "0 1px 0 rgba(255,255,255,0.04) inset, 0 8px 24px -12px rgba(0,0,0,0.6)",
        ring: "0 0 0 1px rgba(125,211,252,0.4), 0 0 0 4px rgba(125,211,252,0.15)",
      },
      borderRadius: {
        xl: "0.875rem",
        "2xl": "1rem",
      },
      keyframes: {
        in: {
          from: { opacity: "0", transform: "translateY(2px)" },
          to: { opacity: "1", transform: "translateY(0)" },
        },
        pulseDot: {
          "0%,100%": { opacity: "0.4" },
          "50%": { opacity: "1" },
        },
      },
      animation: {
        in: "in 180ms ease-out",
        pulseDot: "pulseDot 1.4s ease-in-out infinite",
      },
    },
  },
  plugins: [],
};
