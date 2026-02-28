# Agent Guidelines for Mesh Organiser

This document provides guidelines for AI coding agents working in this repository.

## Workflow Principles

### Always Understand Before Coding
1. Explore the codebase first to understand structure and patterns
2. Read existing similar implementations before starting new work
3. Identify the exact files/locations that need changes
4. Plan the approach before writing code

### Break Large Tasks Into Smaller Steps
- Split complex tasks into discrete, verifiable steps
- Complete and verify each step before moving to the next
- Use a todo list for tracking progress on multi-step tasks

### Verify After Each Change
- Run type checks after making changes
- Test the feature works as expected
- Check for any introduced errors before committing

### Use Specialist Agents When Appropriate
- **@explorer**: Find files, locate code patterns, discover what exists
- **@librarian**: Look up official documentation for libraries/APIs
- **@oracle**: Complex architectural decisions, persistent bugs, high-stakes choices
- **@designer**: UI/UX polish, user-facing visual components
- **@fixer**: Execute well-defined tasks in parallel (3+ independent tasks)

### Parallelize When It Saves Time
- Multiple independent file changes → spawn multiple @fixers
- Research + implementation can run in parallel
- Sequential work must be done serially

## Project Overview

Mesh Organiser is a SvelteKit + Tauri desktop application for organizing 3D print models.
- **Frontend**: Svelte 5, SvelteKit, TypeScript, TailwindCSS v4, Three.js (Threlte)
- **Backend**: Tauri (Rust) with SQLite database
- **Platforms**: Desktop (Windows/macOS/Linux) via Tauri

## Quick Reference

### Commands
See [docs/commands.md](docs/commands.md) for all build, dev, and test commands.

### Code Style
- **Frontend**: See [docs/frontend-style.md](docs/frontend-style.md)
- **Rust**: See [docs/rust-style.md](docs/rust-style.md)

### Git Workflow
See [docs/git-workflow.md](docs/git-workflow.md) for worktree and commit best practices.

## Project Structure

```
src/                    # Frontend source (SvelteKit)
├── lib/               # Shared libraries (api/, components/, utils.ts)
├── routes/            # SvelteKit routes
└── themes/            # CSS themes

src-tauri/             # Tauri desktop app (Rust)
service/               # Standalone service (Rust)
db/                    # Database schema/migrations (Rust)
web/                   # Web server (Rust, optional)

Cargo.toml             # Root workspace (all Rust projects)
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
