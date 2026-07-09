# Build & Development Commands

## Frontend (Node.js)

```bash
npm install          # Install dependencies
npm run dev          # Start full app (Tauri + frontend dev server)
npm run dev:web      # Start frontend-only dev server (browser at localhost)
npm run build        # Build for production
npm run preview      # Preview production build
npm run check        # Type check (run before committing)
npm run check:watch  # Type check with watch mode
npm run lint         # Lint (ESLint)
npm run lint:fix     # Lint and fix
```

## Tauri (Desktop App)

```bash
npm run dev          # Same as tauri dev: full app with hot reload
npm run tauri dev    # Start Tauri dev mode (runs dev:web then opens desktop window)
npm run tauri:build  # Build production desktop app
```

**Official setup:** SSR is disabled via the root layout (`src/routes/+layout.ts` with `export const ssr = false` and `prerender = false`) as in the [Tauri SvelteKit guide](https://v2.tauri.app/start/frontend/sveltekit/). **`@sveltejs/adapter-static`** uses **`fallback: 'index.html'`** so deep links work as a static SPA inside the WebView. See [docs/tauri-sveltekit.md](tauri-sveltekit.md) for port, CSP, and release merge details.

**Release desktop build** applies a stricter CSP via `pnpm run tauri:build` (merges `src-tauri/tauri.release-csp.json`). Use that for production-like verification, not plain `tauri build` without `--config` unless you intentionally want the looser dev CSP string.

### Port 9435 already in use

`pnpm run dev` runs **`dev:cleanup`**: `kill-port 9435` plus a **1 second** pause so the OS can release the socket (Windows often needs this right after the old process exits). If you still see **`Port 9435 is already in use`**, something else is bound to the port: another terminal with **`pnpm run dev:web`** or **`vite preview`**, or a stuck **Node** process. Stop it or run:

`Get-NetTCPConnection -LocalPort 9435 -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }` (PowerShell).

### Dev server hang / `transport invoke timed out` in dev

The UI is **client-only** (`ssr = false`), but SvelteKit’s **dev server** still uses Vite’s internal SSR machinery to boot. If you see **`transport invoke timed out after 60000ms`** or a **blank WebView** until the timeout, the Vite **module runner** is not getting responses (startup ordering, port conflict, antivirus on `node_modules`, or a very busy first compile). Try: free port **9435**, run **`pnpm run dev:web`** and wait until Vite prints **ready**, then run **`pnpm tauri dev`** in another terminal (or use **`pnpm run dev`**, which runs **`dev:cleanup`** then starts Tauri). Reload the window once both are up.

## Rust Backend (entire workspace)

```bash
# Check formatting (entire workspace)
cargo fmt --all -- --check

# Lint (entire workspace - builds implicitly)
cargo clippy --workspace --all-targets
```

### STEP support (cadrum/OCCT)

STEP parsing is provided by [cadrum](https://crates.io/crates/cadrum) and ships in the
Linux and Windows apps (the `step` feature, enabled by `src-tauri`). No system
OpenCASCADE install is needed:

- **Windows**: cadrum downloads a prebuilt, statically linked OCCT — nothing to do.
- **Linux**: cadrum builds OCCT from source with the local toolchain the first time
  (10-30 minutes; needs `cmake` and a C++ compiler). The result is cached in the cargo
  target directory, so later builds are quick. The prebuilt tarball is not used on
  Linux on purpose: its bundled static libstdc++ collides with the system one loaded
  by GTK/WebKit and segfaults the app at startup (see `libmeshthumbnail/Cargo.toml`).

The regular workspace commands cover the STEP path — no special flags or scripts:

```bash
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets
cargo test --workspace --all-targets
```

## Web server (`web` crate)

The Axum **`web`** binary is run directly with Cargo (e.g. `cargo run -p web`). It reads:

| Variable                          | Meaning                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`SERVER_PORT`**                 | TCP listen port; default **`3000`** if unset.                                                                                                                                                                                                                                                                                                                                                                                          |
| **`APP_CONFIG_PATH`**             | Required. Path to the JSON config file (created with defaults if missing).                                                                                                                                                                                                                                                                                                                                                             |
| **`RUST_LOG`**                    | Standard `tracing` / env-filter log level (e.g. `info`, `debug`).                                                                                                                                                                                                                                                                                                                                                                      |
| **`MESH_ORGANISER_CORS_ORIGINS`** | Optional. Comma-separated extra **allowed CORS origins** (whitespace trimmed). If unset or empty after trim, only built-in dev origins are used: `http://localhost:3000` and `http://localhost:5173`. If set, those defaults **remain** and any valid extra origins are added (e.g. `http://192.168.1.10:5173` for LAN Vite). Invalid tokens are skipped with a message on stderr; if every non-empty token is invalid, startup fails. |

## Testing

- **Rust**: `cargo test --workspace` or per-crate (e.g. `cargo test -p service`). Use `crate/tests/` for integration tests.
- **Frontend**: Vitest for unit tests (`src/**/*.test.ts`). Run `npm run test` (or `vitest` / `vitest run file`).

```bash
# Rust
cargo test -p service    # Single crate
cargo test --workspace   # All crates

# Frontend (when configured)
npm run test
vitest run               # Single run
vitest run path/to/file  # Single file
```

## Dependency advisories

Security/maintenance advisories (RUSTSEC/GHSA) in the dependency tree are documented in [docs/advisories.md](advisories.md). Optional `cargo audit` ignore list: `.cargo/audit.toml`.

## Pre-Push / Verification Checklist

Run these before committing, pushing, or claiming work complete. See [AGENTS.md](../AGENTS.md) Pre-Completion Checklist.

### Frontend

```bash
npm run check
npm run lint   # ESLint
npm run test   # when tests exist
```

### Rust (entire workspace)

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace   # or -p <crate> for specific crate
```

STEP is included in the regular workspace commands (see [STEP support](#step-support-cadrumocct)); on Linux the first build compiles OCCT from source via cadrum.
