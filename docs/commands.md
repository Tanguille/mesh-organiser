# Build & Development Commands

## Frontend (Node.js)

```bash
npm install          # Install dependencies
npm run dev         # Start development server
npm run build       # Build for production
npm run preview     # Preview production build
npm run check       # Type check (run before committing)
npm run check:watch # Type check with watch mode
```

## Tauri (Desktop App)

```bash
npm run tauri dev      # Start Tauri dev mode
npm run tauri:build    # Build production desktop app
```

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
npm run test   # when tests exist
```

### Rust (entire workspace)

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features
cargo test --workspace   # or -p <crate> for specific crate
```

To build with STEP, use the [parallelism limits](#step-support-opencascadeocct) so the OCCT build doesn’t freeze the machine.
