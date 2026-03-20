# Agent Guidelines for Mesh Organiser

Guidelines for AI coding agents. Keep this file short and to the point—agents read it at the start of every session.

**Harness principles (2026):** The environment (instructions, checks, context) is what makes agent output reliable. Plan before code, work in small verifiable steps, and always run verification before claiming work complete.

## Workflow Principles

### Always Understand Before Coding

1. Explore the codebase first to understand structure and patterns
2. Read existing similar implementations before starting new work
3. Identify the exact files/locations that need changes
4. Plan the approach before writing code (for large tasks, consider a short spec or implementation plan first)

### Search Before Adding

- Search for existing code, utilities, and patterns before adding new methods, classes, or modules. Avoid duplication.

### Break Large Tasks Into Smaller Steps

- Split complex tasks into discrete, verifiable steps
- Complete and verify each step before moving to the next
- Use a todo list for tracking progress on multi-step tasks

### Verify After Each Change

- Run type checks after making changes
- Test the feature works as expected
- Check for any introduced errors before committing
- **Run `cargo fmt --all` before every push** (enforced in CI)

### Pre-Completion Checklist

Before claiming work complete or ready for review, run verification and only then assert success:

- Frontend: `npm run check`; run relevant tests (`npm run test` if applicable)
- **Frontend (Svelte):** After editing `.svelte` or `.svelte.ts` files, run **svelte-autofixer** (Svelte MCP) on the changed files, fix any reported issues, and run it again to confirm clean. If the Svelte MCP is unavailable, skip this step.
- Rust: `cargo fmt --all`, `cargo clippy --workspace --all-targets`, and tests (e.g. `cargo test -p service`)
- Do not state that tests or checks pass without having run them
- **STEP/OCCT**: Full build compiles OCCT from source. To avoid freezing, limit parallelism (e.g. `CARGO_BUILD_JOBS=2`, Windows: `CL=/MP2`); see [docs/commands.md](docs/commands.md).

### Guardrails

- Code produced by agents must receive human review before merge; do not skip PR review.
- CI (format, lint, tests) is the safety net—ensure changes satisfy it.

### Always Test When Changing Something (Separate Subagent)

- When making behavioural or structural changes (refactors, deduplication, new features), **add or update tests** to guard against regressions.
- **Dangerous edits — test first:** Before doing edits that can change behaviour (control flow, error handling, types, or semantics), **write tests that capture the current intended behaviour**. Run them to establish a baseline, then apply the edit and re-run to confirm behaviour is unchanged. Only then proceed. Examples: replacing `match` with `let`-else, changing `Option`/`Result` handling, refactoring conditionals or casts.
- **Use a separate subagent** dedicated to writing/adding tests rather than having the implementation agent add tests in the same pass. This keeps scope clear and improves test quality.
- The test subagent should: (1) **Explain** what is being tested and why (regression after refactor, new behaviour, etc.), (2) **Plan** concrete test cases (happy path, boundaries, errors), (3) **Execute** (write tests, run test commands, report pass/fail). See [Testing](#testing) below for framework and prompt details.

### Parallelize When It Saves Time

- Multiple independent file changes → spawn multiple @fixers
- Research + implementation can run in parallel
- Sequential work must be done serially

## Project Overview

Mesh Organiser is a SvelteKit + Tauri desktop application for organizing 3D print models.

- **Frontend**: Svelte 5, SvelteKit, TypeScript, TailwindCSS v4, Three.js (Threlte)
- **Backend**: Tauri (Rust) with SQLite database
- **Platforms**: Desktop (Windows/macOS/Linux) via Tauri
- **Tauri + SvelteKit**: Static adapter with SPA **`fallback`**, dev server on **port 9435**, release CSP merge — see [docs/tauri-sveltekit.md](docs/tauri-sveltekit.md)

## Quick Reference

### Key rules

- **Verification before completion**: Run the relevant checks and tests (see [docs/commands.md](docs/commands.md)) before claiming work complete; do not assert success without running them.
- **Search before adding**: Look for existing code and patterns before adding new methods, classes, or modules; reuse instead of duplicating.

### Commands

See [docs/commands.md](docs/commands.md) for all build, dev, and test commands. After making behavioural or structural changes, run tests (e.g. `cargo test -p service`, `npm run test`) and prefer a [separate subagent for adding tests](#always-test-when-changing-something-separate-subagent).

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

## Testing

When changing behaviour or structure (refactors, deduplication, new features), add or update tests and **use a separate subagent** for test writing.

### What we learned (refactor + test subagents)

- **Separate test subagent**: Implementation and test-writing in different subagents improves focus and test quality; the test agent’s only job is to add regression/behavioural coverage.
- **Prompt structure for test agents**: Use a 3-step prompt: (1) **Explain** — what is under test and why (e.g. “regression after consolidating util”); (2) **Plan** — list concrete cases (happy path, boundaries, errors) in a table or list; (3) **Execute** — where to put tests, framework, Arrange–Act–Assert, and the command to run (`cargo test -p service`, `npm run test`).
- **Scope one area per agent**: One subagent per test surface (e.g. util, error serialisation, download, frontend raw_model) so each agent has a clear, small scope.
- **Rust**: Unit tests in `#[cfg(test)] mod tests` next to the code or in `crate/tests/` for integration tests. Use Wiremock (or similar) for HTTP-dependent code so tests don’t hit the network. Run `cargo test -p <crate>`.
- **Frontend**: Vitest for unit tests (`src/**/*.test.ts`). Add `"test": "vitest run"` and `"test:watch": "vitest"` if not present. Run `npm run test`.

### Commands

- **Rust**: `cargo test --workspace` or `cargo test -p service` (see [docs/commands.md](docs/commands.md)).
- **Frontend**: `npm run test` (Vitest).

## Environment Variables

- `VITE_API_PLATFORM`: Set to `"demo"`, `"web"`, or Tauri (default)
- `TAURI_DEV_HOST`: Override host for Tauri development

## Svelte MCP

When working on Svelte or SvelteKit code, use the Svelte MCP server if available:

1. **list-sections** — Call first to discover documentation sections (titles, use_cases, paths).
2. **get-documentation** — Fetch full docs for sections relevant to the task (e.g. runes, load, forms).
3. **svelte-autofixer** — After writing or editing Svelte code, run this and fix any reported issues before finishing.
4. **playground-link** — Only when the user asks for a shareable demo; do not use for code written into the project.

For unfamiliar Svelte 5 or SvelteKit behaviour, use **list-sections** then **get-documentation** before implementing. On every Svelte-related task, run **svelte-autofixer** on changed components before claiming completion (see Pre-Completion Checklist). If the Svelte MCP is unavailable, use the links below and follow [docs/frontend-style.md](docs/frontend-style.md).

## Additional Resources

- [Svelte 5 Runes](https://svelte.dev/blog/runes)
- [SvelteKit Docs](https://kit.svelte.dev/)
- [Tauri Docs](https://tauri.app/)
- [TailwindCSS v4](https://tailwindcss.com/)
- [Threlte (Three.js for Svelte)](https://threlte.xyz/)
