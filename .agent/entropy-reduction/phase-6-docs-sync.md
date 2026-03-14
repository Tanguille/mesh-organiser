# Phase 6: Docs/LLM Sync Check
**Completed:** 2026-03-07

## Findings

### 1. AGENTS.md Verification

**Project Structure:** ✅ **ACCURATE**
- All directories exist: `src/`, `src-tauri/`, `service/`, `db/`, `web/`
- Structure matches actual codebase

**Tauri Commands:** ✅ **ACCURATE**
- Commands located in `src-tauri/src/api/` (verified: blob_api.rs, group_api.rs, label_api.rs, model_api.rs, resource_api.rs, user_api.rs, web_extensions_api.rs)
- `invoke_handler` present in `src-tauri/src/lib.rs` (lines 677-740)
- `mod api;` declaration exists (line 38)

**Common Development Tasks:** ⚠️ **PARTIALLY ACCURATE**
- ✅ Adding Tauri command: Correct about `src-tauri/src/api/` and `invoke_handler`
- ✅ Adding UI component: Correct about `src/lib/components/ui/` and shadcn-svelte pattern
- ⚠️ Adding API endpoint: Says "Export from parent's `index.ts`" but no `index.ts` files exist in API structure. Actual pattern uses dependency injection via `init.ts` files and direct imports

**Environment Variables:** ✅ **ACCURATE**
- `VITE_API_PLATFORM` and `TAURI_DEV_HOST` correctly documented

### 2. docs/commands.md Verification

**NPM Commands:** ✅ **ACCURATE**
All commands match `package.json`:
- `npm run dev` ✅
- `npm run build` ✅
- `npm run preview` ✅
- `npm run check` ✅
- `npm run check:watch` ✅
- `npm run tauri` ✅
- `npm run tauri:build` ✅ (uses `--no-sign` flag correctly)

**Cargo Commands:** ✅ **ACCURATE**
All commands are valid for the workspace structure defined in root `Cargo.toml`

### 3. docs/frontend-style.md Verification

**TypeScript:** ✅ **ACCURATE**
- Strict mode implied by `svelte-check` usage
- Interfaces used for objects (verified in `src/lib/api/shared/`)
- Types used for unions (e.g., `FileType` in blob_api.ts)

**Svelte 5 Runes:** ✅ **ACCURATE**
- `$state`, `$derived`, `$effect` used throughout (verified in `src/routes/+page.svelte`)
- `$props()` used in components (verified in `src/lib/components/ui/button/button.svelte`)
- `$bindable()` used (verified in button.svelte line 49)

**Imports & Path Aliases:** ⚠️ **INCOMPLETE**
- ✅ `$lib` alias correctly documented
- ❌ Missing: `@/*` alias also defined in `svelte.config.js` (line 13)

**Naming Conventions:** ✅ **ACCURATE**
- Files: kebab-case (verified: `model-api.ts`, `three-d-scene.svelte`)
- Components: PascalCase (verified: `ModelGrid.svelte` pattern)
- Functions: camelCase (verified: `loadModelAutomatically` in utils.ts)

**TailwindCSS & cn():** ✅ **ACCURATE**
- `cn()` function exists in `src/lib/utils.ts` (lines 7-9)
- Used throughout components (verified in button.svelte)

### 4. docs/rust-style.md Verification

**General Conventions:** ✅ **ACCURATE**
- `#[tauri::command]` used correctly (verified in model_api.rs)
- sqlx used for database (verified in Cargo.toml workspace deps)
- Clippy lints enabled in workspace (verified in root Cargo.toml lines 8-14)

**Error Handling:** ✅ **ACCURATE**
- `ApplicationError` enum exists in `src-tauri/src/error.rs`
- `?` operator used correctly (verified in model_api.rs line 39)
- Pattern matches documentation example exactly

**Commands:** ✅ **ACCURATE**
All cargo commands are valid for workspace structure

### 5. docs/git-workflow.md Verification

**Worktree Workflow:** ✅ **ACCURATE**
- Commands are valid git worktree commands
- Pattern promotes clean branch workflow

**Dependency Files:** ✅ **ACCURATE**
- All listed files should not be committed unless updating dependencies
- `mise.toml` correctly included

### 6. Schema/DB Documentation

**No separate schema documentation found** to compare against `db/migrations/`

Migrations exist and are valid:
- `20250317142939_create_initial_tables.sql` - Initial schema
- `20250406152932_add_flags_to_model.sql` - Model flags
- `20250426102009_add_sublabels.sql` - Label hierarchy
- `20250530150713_add_resources_table.sql` - Resources
- `20250814174529_add_keywords_to_labels.sql` - Keywords
- `20251027162513_multi_user.sql` - Multi-user support
- `20251206000641_blob_via_file_path.sql` - Blob storage path

## Sync Issues

| Type | File | Issue | Current State | Expected State |
|------|------|-------|---------------|----------------|
| Medium | AGENTS.md | API export pattern incorrect | Says "Export from parent's index.ts" | Should say "Use dependency injection via init.ts files" |
| Low | docs/frontend-style.md | Missing path alias | Only documents `$lib` | Should also document `@/*` alias from svelte.config.js |
| Low | AGENTS.md | Missing `@/*` alias reference | Only mentions `$lib` | Should mention both aliases |

## Action Items
- [ ] Fix AGENTS.md "Adding a new API endpoint" section to reflect actual dependency injection pattern using `init.ts` files instead of `index.ts` exports
- [ ] Add `@/*` alias documentation to docs/frontend-style.md and AGENTS.md
- [ ] Consider adding brief schema documentation to db/ folder referencing migration files

## Summary Stats
- Total issues: 3
- Critical: 0 | High: 0 | Medium: 1 | Low: 2

---

## Detailed Verification Notes

### Project Structure Verified
```
src/                    ✅ Exists with lib/, routes/, themes/
src-tauri/             ✅ Exists with src/api/, Cargo.toml
service/               ✅ Exists with src/, Cargo.toml
db/                    ✅ Exists with migrations/, src/, Cargo.toml
web/                   ✅ Exists with README.md, Cargo.toml
```

### Tauri Commands in invoke_handler (src-tauri/src/lib.rs:677-740)
All 63 commands properly registered:
- api::add_model, api::get_models, api::get_labels, etc.
- open_in_slicer, get_initial_state, download_file, etc.
- get_configuration, set_configuration, compute_model_folder_size

### Frontend Patterns Verified
- Runes: `$state`, `$derived`, `$effect`, `$props`, `$bindable` all used
- cn() utility: Implemented in utils.ts using clsx + tailwind-merge
- Naming: All conventions followed consistently

### Rust Patterns Verified
- Import order: std → crate → external → db/service → tauri (consistent)
- Error handling: ApplicationError with #[from] derives
- Commands: All use #[tauri::command] with proper error propagation
