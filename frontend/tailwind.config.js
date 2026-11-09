/** @type {import('tailwindcss').Config} */

// Dracula 色板与 src/theme.ts 保持一致
const dracula = {
  bg: "#282a36",
  "bg-dark": "#21222c",
  "bg-page": "#191a21",
  input: "#1e2029",
  border: "#44475a",
  fg: "#f8f8f2",
  comment: "#6272a4",
  purple: "#bd93f9",
  pink: "#ff79c6",
  cyan: "#8be9fd",
  red: "#ff5555",
  green: "#50fa7b",
  yellow: "#f1fa8c",
};

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: Object.fromEntries(
        Object.entries(dracula).map(([k, v]) => [`dracula-${k}`, v]),
      ),
    },
  },
  plugins: [],
};
