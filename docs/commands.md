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
**No test framework is currently configured.** If adding tests:
- Use Vitest for unit tests (Svelte/TypeScript)
- Use Playwright for E2E tests

```bash
vitest              # Run all tests
vitest run file     # Run single test file
vitest              # Run tests in watch mode
```

## Pre-Push Checks

Run these before committing/pushing:

### Frontend
```bash
npm run check
```

### Rust (entire workspace)
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
```
