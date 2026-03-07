# Phase 1: Architecture Review
**Completed:** 2026-03-07

## Overview
Mesh Organiser is a SvelteKit + Tauri desktop application for organizing 3D print models. The codebase follows a workspace architecture with clear separation between frontend, Tauri bridge, business logic service layer, and database layer.

## Findings

### 1. Folder Structure Alignment ✓
The folder structure matches AGENTS.md specification:
- `src/` — Frontend (Svelte 5, SvelteKit, TypeScript, TailwindCSS v4, Three.js/Threlte)
- `src-tauri/` — Tauri desktop app (Rust) with api/ subfolder
- `service/` — Standalone business logic service (Rust)
- `db/` — Database layer with SQLite and sqlx (Rust)
- `web/` — Optional web server (Rust, axum-based)

### 2. Layer Separation Assessment

#### Crate Dependencies (Correct Direction)
```
db (lowest) → service → src-tauri
                ↓
               web
```

- **db/**: Pure database layer, depends only on sqlx and basic crates. No circular deps.
- **service/**: Business logic layer, depends on `db`. Contains app_state, import/export services.
- **src-tauri/**: Bridge layer, depends on `db` and `service`. API handlers are generally thin.
- **web/**: Web server, depends on `db` and `service` (same patterns as Tauri).

**No circular dependencies detected** between workspace crates.

#### API Layer Thinness
The Tauri API handlers (`src-tauri/src/api/`) are reasonably thin:
- Most delegate to `service::` or `db::` functions
- Business logic is not duplicated in API layer
- Exception: `web_extensions_api.rs` (403 lines) contains significant logic including HTTP downloading and ZIP handling — should be moved to service layer

### 3. Frontend Organization

**Structure (SvelteKit conventions):**
- `src/routes/` — SvelteKit routes (14 route folders)
- `src/lib/api/` — API abstraction layer with DI container
- `src/lib/components/` — UI components (app-sidebar.svelte, edit/, view/, ui/)
- `src/lib/utils.ts` — Shared utilities

**API Abstraction Pattern:**
- Clean abstraction with interface segregation (`IBlobApi`, `IGroupApi`, etc.)
- Dependency injection container (`dependency_injection.ts`)
- Multiple backend implementations: `demo/`, `tauri/`, `web/`, `tauri-online/`, `tauri-sync/`

### 4. God Files & Functions

#### Rust Files >500 Lines (8 files)
| Lines | File | Notes |
|-------|------|-------|
| 753 | `src-tauri/src/lib.rs` | Main entry point with Tauri setup — acceptable |
| 668 | `service/src/import_service.rs` | Complex import logic — consider splitting |
| 516 | `db/src/label_db.rs` | Database operations — reasonable |
| 458 | `src-tauri/src/service/import_service.rs` | **Duplicated service logic** — remove |
| 424 | `web/src/controller/model_controller.rs` | Web controller — acceptable |
| 415 | `db/src/model_db.rs` | Database operations — reasonable |
| 403 | `src-tauri/src/api/web_extensions_api.rs` | API contains business logic — extract |
| 355 | `web/src/controller/blob_controller.rs` | Web controller — acceptable |

#### Frontend Files >300 Lines (6 files)
| Lines | File | Notes |
|-------|------|-------|
| 514 | `src/routes/settings/+page.svelte` | Settings page — acceptable |
| 472 | `src/lib/components/app-sidebar.svelte` | Main sidebar — acceptable |
| 469 | `src/lib/components/view/group-grid.svelte` | Grid component — acceptable |
| 449 | `src/lib/components/edit/model.svelte` | Model editor — acceptable |
| 355 | `src/lib/api/tauri/tauri_import.ts` | Import API wrapper — acceptable |
| 354 | `src/lib/components/edit/multi-model.svelte` | Multi-model editor — acceptable |

#### God Functions (Rust)
| Function | Lines | File | Issue |
|----------|-------|------|-------|
| `new_window_with_url` | ~150 | `src-tauri/src/lib.rs:251` | Single function does window creation, menu building, navigation, download handling |
| `import_path_inner` | ~64 | `service/src/import_service.rs:99` | Complex recursive import logic — acceptable with async |
| `run()` | ~230 | `src-tauri/src/lib.rs:520` | Main app setup — acceptable for entry point |

### 5. Separation of Concerns Issues

#### Issue 1: Duplicate Service Code
**Location:** `src-tauri/src/service/import_service.rs` (458 lines)
**Problem:** Duplicate of `service/src/import_service.rs` logic in Tauri crate
**Severity:** High
**Recommendation:** Remove duplicate and use service crate directly

#### Issue 2: Business Logic in API Layer
**Location:** `src-tauri/src/api/web_extensions_api.rs` (403 lines)
**Problem:** Contains HTTP download logic, ZIP creation, file handling that belongs in service layer
**Severity:** Medium
**Recommendation:** Extract to `service/src/web_extension_service.rs`

#### Issue 3: Monolithic lib.rs
**Location:** `src-tauri/src/lib.rs` (753 lines)
**Problem:** Contains window management, deep link parsing, configuration reading, and main app setup
**Severity:** Low
**Recommendation:** Consider extracting window management to `src-tauri/src/window_manager.rs`

## Issues Found

| Severity | Location | Issue | Recommendation |
|----------|----------|-------|----------------|
| High | `src-tauri/src/service/import_service.rs` | Duplicate service code exists in Tauri crate | Remove; use service crate directly |
| Medium | `src-tauri/src/api/web_extensions_api.rs` | Business logic (HTTP, ZIP) in API layer | Extract to service layer |
| Medium | `src-tauri/src/lib.rs:251` | `new_window_with_url` is a god function | Extract window builder to separate module |
| Low | `src-tauri/src/lib.rs` | Main lib.rs is 753 lines | Consider extracting window/deep-link modules |
| Low | `db/src/label_db.rs` | 516 lines of DB operations | Acceptable; could split if grows larger |
| Info | `service/src/import_service.rs` | 668 lines of import logic | Consider splitting into submodules |

## Summary Stats

### Codebase Size
- **Rust:** ~9,000 lines (excluding patch/)
- **Frontend:** ~16,500 lines (Svelte + TypeScript)
- **Total:** ~25,500 lines

### Crate Dependency Map
```
┌─────────────┐     ┌──────────────┐
│   db (db)   │◄────│ service (biz)│
└─────────────┘     └──────┬───────┘
                           │
         ┌─────────────────┼─────────────────┐
         │                 │                 │
         ▼                 ▼                 ▼
┌─────────────────┐ ┌──────────────┐ ┌──────────────┐
│  src-tauri      │ │     web      │ │  (others)    │
│  (Tauri bridge) │ │ (Web server) │ │              │
└─────────────────┘ └──────────────┘ └──────────────┘
```

### Issue Count
- **Total issues:** 6
- **Critical:** 0
- **High:** 1
- **Medium:** 2
- **Low:** 2
- **Info:** 1

## Action Items

- [ ] Remove duplicate `src-tauri/src/service/import_service.rs` and use service crate
- [ ] Extract `web_extensions_api.rs` business logic to service layer
- [ ] Refactor `new_window_with_url` into window management module
- [ ] Document API abstraction pattern for new contributors
- [ ] Consider adding architecture decision record (ADR) for crate boundaries

## Architecture Strengths

1. **Clean crate separation** — No circular dependencies
2. **Good abstraction layers** — API layer is mostly thin
3. **Frontend DI pattern** — Clean interface-based API abstraction
4. **Consistent patterns** — Both Tauri and web follow same service/db patterns
5. **Workspace organization** — Shared dependencies in root Cargo.toml
