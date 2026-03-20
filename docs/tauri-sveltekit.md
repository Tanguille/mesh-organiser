# Tauri 2 + SvelteKit conventions (Mesh Organiser)

This repo follows the [Tauri 2 SvelteKit guide](https://v2.tauri.app/start/frontend/sveltekit/) with a few project-specific choices.

## Dev server and port

- **Vite** listens on **`http://127.0.0.1:9435`** with **`strictPort: true`** when **`TAURI_DEV_HOST`** is unset ([`vite.config.js`](../vite.config.js)), matching Tauri’s **`devUrl`** probe. That avoids a **blank WebView** (hostname/`::1` mismatch) and avoids **`tauri dev`** hanging on “Waiting for … `127.0.0.1:9435`” when Vite was only bound in a way that did not accept IPv4 loopback.
- **`src-tauri/tauri.conf.json`** sets **`devUrl`** to **`http://127.0.0.1:9435`**. Override **`TAURI_DEV_HOST`** for LAN/device testing and align **`devUrl`**, HMR, and CSP as described below.
- **`beforeDevCommand`** is `pnpm run dev:web`.
- **`pnpm run dev`** runs **`dev:cleanup`** (`kill-port 9435`, then **1s** pause for the OS to release the port) then **`tauri dev`**, so a stuck process on 9435 is less likely to block startup.

### Debugging a blank window in dev

1. Confirm the UI loads in a normal browser at **`http://127.0.0.1:9435/`** while `pnpm run dev:web` is running. The app uses **`isTauri()`** from `@tauri-apps/api/core`: in a normal browser (no WebView IPC) it initializes **demo** APIs instead of calling **`invoke`**, so you should not see `Cannot read properties of undefined (reading 'invoke')` there.
2. Open **WebView DevTools** (Windows: **Ctrl+Shift+I** with the app focused). Check **Console** for uncaught errors or failed dynamic imports; check **Network** for a failed document request or blocked scripts.
3. If the page loads but stays on the spinner, errors from `initApi()` / Tauri **`invoke`** should appear in the console; the root layout also surfaces initialization failures via **toast**.

**Slow first load / “optimized dependencies changed. reloading”:** Vite may pre-bundle a large set of imports on first open. [`vite.config.js`](../vite.config.js) uses **`optimizeDeps.include`** (Lucide icon subtree, Tauri API/plugins, common deps) and **`server.warmup.clientFiles`** on the root layout/page to reduce mid-session dep-cache invalidation and full reloads that can race SvelteKit’s dynamic imports.

### Vite: `transport invoke timed out after 60000ms` (SSR / `fetchModule`)

If the terminal shows **`(ssr) Error when evaluating SSR module`** … **`transport invoke timed out after 60000ms`**, or the browser **never finishes loading** the first document while Vite says **ready**, Vite’s SSR **ModuleRunner** RPC may be taking longer than the default **60s** per `fetchModule` (common on Windows + long `pnpm` paths + AV scanning). This is separate from SQLite/Tauri Rust.

**This repo** applies a **`pnpm` patch** on [`vite@8.0.1`](../patches/vite@8.0.1.patch) (see [`package.json`](../package.json) `pnpm.patchedDependencies`) so the SSR transport uses a **300s** default `fetchModule` timeout instead of 60s. Override with **`VITE_SSR_FETCH_MODULE_TIMEOUT_MS`** if needed (minimum **60000**).

Also try:

1. **`server.warmup.ssrFiles`** in [`vite.config.js`](../vite.config.js) to front-load SvelteKit SSR transforms.
2. Delete **`node_modules/.vite`** and run **`pnpm run dev:web -- --force`**.
3. Exclude the repo and **`pnpm store path`** from aggressive real-time AV scanning.
4. If you use **pnpm’s global virtual store**, extend Vite **`server.fs.allow`** ([docs](https://vite.dev/config/server-options.html#server-fs-allow); [sveltejs/kit#14162](https://github.com/sveltejs/kit/issues/14162)).
5. If problems persist on **Node 25+**, try **Node 22 LTS** (tooling is often validated there first).

## SPA shell vs prerender

- Root [`src/routes/+layout.ts`](../src/routes/+layout.ts) uses **`export const ssr = false`** (no Node SSR in the WebView).
- **`@sveltejs/adapter-static`** is configured with **`fallback: 'index.html'`** in [`svelte.config.js`](../svelte.config.js) so **deep links** (e.g. `/group/123`, `/share/...`) work when served as static files inside Tauri.
- Dynamic `[slug]` routes do **not** use stub `entries()` for build-time paths; the client router resolves params at runtime.

## Content-Security-Policy (dev vs release)

- **Development** CSP lives in [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json): Vite + HMR on **`127.0.0.1:9435`** and **localhost:9435** (explicit `ws://127.0.0.1:9435` / `ws://localhost:9435`), **`unsafe-eval`** for tooling, and **`img-src`** allowing **`https:`** (no `*`).
- **Release** builds merge [`src-tauri/tauri.release-csp.json`](../src-tauri/tauri.release-csp.json), which drops dev-only origins, omits **`unsafe-eval`**, and tightens **`connect-src`** / **`img-src`** while keeping **`ipc:`** / **`http://ipc.localhost`** and **`asset:`** / **`http://asset.localhost`** for Tauri IPC and `convertFileSrc`-style URLs.

Local desktop release build:

```bash
pnpm run tauri:build
```

That runs `tauri build --no-sign --config src-tauri/tauri.release-csp.json` ([`package.json`](../package.json)). CI release jobs pass the same `--config` via [`release.yaml`](../.github/workflows/release.yaml).

If a release build breaks (e.g. CSP blocks a real `fetch` or worker), add the **minimal** directive to `tauri.release-csp.json` and document why.

## `TAURI_DEV_HOST` (optional)

If you point the dev server at a non-localhost interface (LAN / device testing), you must align **`devUrl`**, Vite **`server.host` / HMR**, and **both** CSP files. The default workflow is **localhost-only**.

## Multi-target frontend

The same SvelteKit app can run as **Tauri**, **web**, or **web_share** (see `initApi` / DI). Do not import `@tauri-apps/*` from **`src/lib/api/shared/`**; keep platform code behind the existing init boundaries.
