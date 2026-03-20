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
cargo clippy --workspace --all-targets --all-features
```

### STEP support (OpenCascade/OCCT)

The **`step`** feature builds OpenCascade/OCCT from C++ source the first time. That build is heavy and can freeze the PC if too many compiler processes run at once. **Limit parallelism** so the machine stays responsive:

**PowerShell (Windows):** from repo root you can run:

```powershell
.\scripts\build-with-step.ps1
```

Or set the env vars yourself: `CARGO_BUILD_JOBS=2`, `CL=/MP2`; if CMake errors about "Compatibility with CMake < 3.5", set `CMAKE_POLICY_VERSION_MINIMUM=3.5`.

**Bash (Linux/macOS):**

```bash
CARGO_BUILD_JOBS=2 cargo build --workspace --all-targets --all-features
```

After OCCT has built once, later builds are quick (Cargo caches it). For day-to-day work without STEP you can omit `--all-features`:

```bash
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets
cargo test --workspace --all-targets
```

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
cargo clippy --workspace --all-targets --all-features
cargo test --workspace   # or -p <crate> for specific crate
```

To build with STEP, use the [parallelism limits](#step-support-opencascadeocct) so the OCCT build doesn’t freeze the machine.
