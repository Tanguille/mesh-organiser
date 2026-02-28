# Agent Guidelines for Mesh Organiser

This document provides guidelines for AI coding agents working on the Mesh Organiser codebase.

## Project Overview

Mesh Organiser is a SvelteKit + Tauri desktop application for organizing 3D print models.
- **Frontend**: Svelte 5, SvelteKit, TypeScript, TailwindCSS v4, Three.js (Threlte)
- **Backend**: Tauri (Rust) with SQLite database
- **Platforms**: Desktop (Windows/macOS/Linux) via Tauri

## Build & Development Commands

### Frontend (Node.js)
```bash
npm install          # Install dependencies
npm run dev         # Start development server
npm run build       # Build for production
npm run preview     # Preview production build
npm run check       # Type check (run before committing)
npm run check:watch # Type check with watch mode
```

### Tauri (Desktop App)
```bash
npm run tauri dev      # Start Tauri dev mode
npm run tauri:build    # Build production desktop app
```

### Testing
**No test framework is currently configured.** If adding tests:
- Use Vitest for unit tests (Svelte/TypeScript)
- Use Playwright for E2E tests

```bash
vitest              # Run all tests
vitest run file     # Run single test file
vitest              # Run tests in watch mode
```

## Pre-Push Checks

Before committing/pushing, run these checks locally:

### Frontend
```bash
npm run check       # TypeScript type checking (svelte-check)
```

### Rust Backend (src-tauri/)
```bash
cd src-tauri
cargo fmt --all -- --check    # Check formatting
cargo clippy --workspace --all-targets --all-features  # Lint (builds implicitly)
```

## Git Workflow Best Practices

### Use a Fresh Worktree for Each Task

To avoid accidentally committing unrelated changes, create a new git worktree for each task:

```bash
# Create a new worktree from main for your task
git worktree add ../mesh-organiser-task -b feature/your-task-name

# Work on your task in the new directory
cd ../mesh-organiser-task

# When done, delete the worktree and branch
cd ..
git worktree remove mesh-organiser-task
git branch -D feature/your-task-name
```

This ensures:
- Each task starts from a clean main branch state
- No uncommitted or unrelated changes leak into your commits
- Multiple tasks can be worked on in parallel without interference

**Do NOT** commit changes to package.json, package-lock.json, Cargo.lock, mise.toml, or other dependency files unless the task explicitly involves updating dependencies.

## Code Style Guidelines

### TypeScript
- **Strict mode enabled**: All TypeScript must pass strict type checking
- Use explicit type annotations for function parameters/returns when not inferrable
- Prefer interfaces over types for object shapes; use `type` for unions/intersections

```typescript
// Good - explicit types
function loadModelAutomatically(configuration: Configuration, model: Model): boolean { }

// Good - interface for objects
interface Model { id: number; name: string; blob: Blob; group?: ModelGroup; }

// Good - type for unions
type FileType = 'stl' | 'obj' | '3mf' | 'step' | 'gcode';
```

### Svelte 5
- Use runes (`$state`, `$derived`, `$effect`) for reactive state
- Prefer `.svelte` files for components, `.ts` files for logic

```svelte
<script lang="ts">
    let count = $state(0);
    let doubled = $derived(count * 2);
</script>
<button onclick={() => count++}>{doubled}</button>
```

### Imports & Path Aliases
Defined in `svelte.config.js`:
```typescript
import { cn } from '$lib/utils';           // $lib -> ./src/lib
import { Model } from '$lib/api/shared/model_api';
```

### Naming Conventions
- **Files**: kebab-case (`model-api.ts`, `three-d-scene.svelte`)
- **Components**: PascalCase (`ModelGrid.svelte`, `Button.svelte`)
- **Types/Interfaces**: PascalCase (`Model`, `Configuration`)
- **Functions**: camelCase (`loadModelAutomatically`)
- **Constants**: SCREAMING_SNAKE_CASE for config, camelCase for others
- **CSS Classes**: kebab-case (Tailwind standard)

### Error Handling
**TypeScript/JavaScript**: Use try/catch for async, throw descriptive errors
```typescript
try {
    const response = await fetch('/api/models');
    if (!response.ok) throw new Error(`Failed to fetch: ${response.status}`);
    return await response.json();
} catch (error) {
    console.error('Failed to fetch models:', error);
    throw error;
}
```

**Rust (Tauri)**: Use `ApplicationError` enum, `?` operator for propagation
```rust
#[tauri::command]
async fn get_models(state: State<'_, TauriAppState>) -> Result<Vec<Model>, ApplicationError> {
    let models = model_db::get_all_models(&state.app_state.db).await?;
    Ok(models)
}
```

### TailwindCSS
Use utility classes; use `cn()` from `$lib/utils` for conditional classes
```typescript
import { cn } from '$lib/utils';
<div class={cn("base-class", isActive && "active-class")}>
```

### Rust Backend (src-tauri/)
- Follow standard Rust conventions (rustfmt auto-formats)
- Use `#[tauri::command]` for Tauri command handlers
- Use sqlx for database queries; enable clippy lints (checked in CI)

## Project Structure
```
src/                    # Frontend source
├── lib/               # Shared libraries (api/, components/, utils.ts)
├── routes/            # SvelteKit routes
└── themes/           # CSS themes

src-tauri/             # Rust backend
├── src/api/           # Tauri command handlers
├── src/service/       # Business logic
└── src/lib.rs         # Main Tauri app
```

## Common Development Tasks

### Adding a new API endpoint (Frontend)
1. Create API function in appropriate file under `src/lib/api/`
2. Export from parent's `index.ts`
3. Use existing patterns (e.g., `src/lib/api/shared/model_api.ts`)

### Adding a new Tauri command
1. Add command function in `src-tauri/src/api/`
2. Register in `src-tauri/src/lib.rs` `invoke_handler`
3. Create TypeScript wrapper in `src/lib/api/tauri/`

### Adding a new UI component
1. Use existing shadcn-svelte components as reference
2. Store in `src/lib/components/ui/`
3. Follow the `index.ts` + component pattern

## Environment Variables
- `VITE_API_PLATFORM`: Set to `"demo"`, `"web"`, or Tauri (default)
- `TAURI_DEV_HOST`: Override host for Tauri development

## Additional Resources
- [Svelte 5 Runes](https://svelte.dev/blog/runes)
- [SvelteKit Docs](https://kit.svelte.dev/)
- [Tauri Docs](https://tauri.app/)
- [TailwindCSS v4](https://tailwindcss.com/)
- [Threlte (Three.js for Svelte)](https://threlte.xyz/)
