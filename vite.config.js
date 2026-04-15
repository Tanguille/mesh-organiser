import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig, loadEnv } from "vite";

const packageJsonPath = fileURLToPath(
  new URL("./package.json", import.meta.url),
);
const packageVersion = JSON.parse(
  readFileSync(packageJsonPath, "utf-8"),
).version;

/** Keep in sync with `devUrl` in `src-tauri/tauri.conf.json` (Tauri probes this before opening the WebView). */
const DEV_PORT = 9435;

// `host: false` / hostname mismatches can bind in ways that do not answer `127.0.0.1`, so `tauri dev` hangs on
// "Waiting for your frontend dev server to start on http://127.0.0.1:9435/".
const devHost = process.env.TAURI_DEV_HOST ?? "127.0.0.1";

const isTauriCliBuild = process.env.TAURI_ENV_PLATFORM != null;

/** @type {false | 'esbuild'} */
const tauriMinify = process.env.TAURI_ENV_DEBUG ? false : "esbuild";

// https://vitejs.dev/config/
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "VITE_");
  const fromEnv =
    typeof env.VITE_APP_VERSION === "string" ? env.VITE_APP_VERSION.trim() : "";
  const viteAppVersion = fromEnv.length > 0 ? fromEnv : `v${packageVersion}`;

  return {
    plugins: [tailwindcss(), sveltekit()],

    define: {
      "import.meta.env.VITE_APP_VERSION": JSON.stringify(viteAppVersion),
    },

    // Pre-bundle deep imports so Vite does not discover them mid-session, invalidate the dep
    // cache, and full-reload (which races dynamic imports like `.svelte-kit/generated/client/nodes/0.js`).
    // Three/Threlte are large; without explicit `include`, first load can block ~1min then reload.
    optimizeDeps: {
      include: [
        "@lucide/svelte/icons/**",
        "@tauri-apps/api/app",
        "@tauri-apps/api/core",
        "@tauri-apps/api/event",
        "@tauri-apps/api/path",
        "@tauri-apps/api/webview",
        "@tauri-apps/api/window",
        "@tauri-apps/plugin-dialog",
        "@tauri-apps/plugin-fs",
        "@tauri-apps/plugin-http",
        "@tauri-apps/plugin-os",
        "@tauri-apps/plugin-process",
        "@tauri-apps/plugin-updater",
        "bits-ui",
        "fflate",
        "mode-watcher",
        "qs",
        "svelte-sonner",
        "tailwind-merge",
        "tailwind-variants",
        "three",
        "@threlte/core",
        "@threlte/extras",
        "three/examples/jsm/loaders/OBJLoader.js",
        "three/examples/jsm/loaders/STLLoader.js",
        "three/examples/jsm/utils/BufferGeometryUtils.js",
        "threejs-webworker-3mf-loader",
      ],
    },

    // Client: `import.meta.env`; Tauri sets `TAURI_ENV_*` during `tauri build` / `tauri dev`.
    envPrefix: ["VITE_", "TAURI_ENV_*"],

    test: {
      include: ["src/**/*.{test,spec}.{js,ts}"],
    },

    // Prevent Vite from clearing the terminal (Rust/compiler errors stay visible).
    clearScreen: false,

    server: {
      // Pre-transform SvelteKit’s SSR entry graph at dev-server start so the first request does not
      // spend minutes in esbuild/oxc on a cold Windows + pnpm tree — Vite 8’s SSR ModuleRunner RPC
      // times out after 60s per `fetchModule` (`transport invoke timed out`).
      warmup: {
        ssrFiles: [
          "node_modules/@sveltejs/kit/src/runtime/server/index.js",
          "node_modules/@sveltejs/kit/src/utils/promise.js",
        ],
        clientFiles: [
          "./src/routes/+layout.svelte",
          "./src/routes/+page.svelte",
          "./src/lib/components/view/three-d-canvas.svelte",
          "./src/lib/components/view/three-d-scene.svelte",
          "./src/lib/workers/parseModelWorker.ts",
        ],
      },
      port: DEV_PORT,
      strictPort: true,
      host: devHost,
      hmr:
        process.env.TAURI_DEV_HOST != null
          ? {
              protocol: "ws",
              host: process.env.TAURI_DEV_HOST,
              port: 1421,
            }
          : undefined,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },

    // Only when invoked via `tauri build` (plain `vite build` keeps default targets for static/web deploys).
    ...(isTauriCliBuild
      ? {
          build: {
            target:
              process.env.TAURI_ENV_PLATFORM === "windows"
                ? "chrome105"
                : "safari13",
            minify: tauriMinify,
            sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
          },
        }
      : {}),
  };
});
