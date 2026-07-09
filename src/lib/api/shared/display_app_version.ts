/**
 * Normalizes Vite `VITE_APP_VERSION` for demo/web hosts.
 * Trimming matches `vite.config.js` so empty/whitespace env falls back like the bundle default.
 */
export function displayAppVersion(viteAppVersion: unknown): string {
  const s = typeof viteAppVersion === "string" ? viteAppVersion.trim() : "";

  if (s.length > 0) {
    return s;
  }

  return "dev";
}
