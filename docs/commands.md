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
