# Build & Development Commands

## Frontend

```bash
pnpm install          # Install dependencies
pnpm run dev          # Start full app (Tauri + frontend dev server)
pnpm run dev:web      # Start frontend-only dev server (browser at localhost)
pnpm run build        # Build for production
pnpm run preview      # Preview production build
pnpm run check        # Type check (run before committing)
pnpm run check:watch  # Type check with watch mode
pnpm run lint         # Lint (oxlint)
pnpm run lint:fix     # Lint and fix
pnpm run format       # Format (oxfmt)
pnpm run format:check # Format check (oxfmt, as in CI)
```

## Tauri (Desktop App)

```bash
pnpm tauri dev       # Start Tauri dev mode (runs dev:web then opens desktop window)
pnpm run tauri:build # Build production desktop app with the release CSP
```

Dev port, SPA/SSR setup, release CSP, and dev-server troubleshooting (port **9435** in use, `transport invoke timed out`): see [docs/tauri-sveltekit.md](tauri-sveltekit.md).

## Rust Backend (entire workspace)

```bash
# Check formatting (entire workspace)
cargo fmt --all -- --check

# Lint (entire workspace - builds implicitly)
cargo clippy --workspace --all-targets
```

### STEP support (cadrum/OCCT)

STEP parsing is provided by [cadrum](https://crates.io/crates/cadrum) and ships in the
Linux, Windows, and macOS apps (the `step` feature, enabled by `src-tauri`). No system
OpenCASCADE install is needed:

- **All platforms**: cadrum's build script downloads a prebuilt, statically linked OCCT
  for the current target automatically (see cadrum's README for the target table).
- **Linux** is the one exception: it builds OCCT from source with the local toolchain
  the first time (10-30 minutes; needs `cmake` and a C++ compiler; cached in the cargo
  target directory afterwards). The prebuilt is skipped on purpose: its bundled static
  libstdc++ collides with the system one loaded by GTK/WebKit and segfaults the app at
  startup (see `libmeshthumbnail/Cargo.toml`).

New stored `.stp` imports use `.stp.zip`. Legacy stored `.stp` blobs can contain ZIP
bytes despite their old file type, and their metadata is not migrated automatically.
Re-importing alone reuses the existing hash and leaves that storage unchanged. Keep
the original source file before permanently removing affected models; re-import
only after every reference to the old blob is gone and blob cleanup has removed it.
In-place `.stp` imports retain their original plain format and are unaffected.

The regular workspace commands cover the STEP path — no special flags or scripts:

```bash
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets
cargo test --workspace --all-targets
```

## Testing

```bash
# Rust
cargo test -p service             # Single crate
cargo test --workspace            # All crates

# Frontend
pnpm run test                     # Single run
pnpm run test:watch               # Watch mode
pnpm exec vitest run path/to/file # Single file
```
