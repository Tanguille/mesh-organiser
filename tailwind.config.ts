import tailwindcssAnimate from "tailwindcss-animate";

/** Theme (colors, radius, keyframes, etc.) lives in src/app.css via @theme. This file only registers the animate plugin and dark mode. */
export default {
  darkMode: ["class"],
  content: ["./src/**/*.{html,js,svelte,ts}"],
  safelist: ["dark"],
  plugins: [tailwindcssAnimate],
};
