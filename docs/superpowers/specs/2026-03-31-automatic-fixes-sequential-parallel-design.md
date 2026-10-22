# Automatic Fixes Application Design Spec

> **Goal:** Apply all 14 automatic fixes from the code review findings using sequential category processing with parallel file edits within each category.

## Overview

We have 14 automatic fixes across 4 categories: error-handling (4), duplication (5), memory (3), naming (2). These fixes are safe to apply automatically (refactoring-preserving). We'll process categories sequentially to run tests between them, and within each category dispatch parallel subagents for independent file groups to maximize speed.

## Architecture

### Components

1. **Category Sequencer**: Orchestrates the order of categories: error-handling → duplication → memory → naming. Ensures tests pass between categories.
2. **File Grouper**: Analyzes fixes within a category to group by independent files/modules (no overlapping changes). Uses dependency analysis to avoid conflicts.
3. **Subagent Dispatcher**: Launches parallel subagents for each file group, providing exact fixes for those files.
4. **Test Runner**: Executes frontend and backend test suites after each category.
5. **Commit Manager**: Commits fixes for each category with conventional commit messages.

### Data Flow

```
1. Load PRIORITIZED_FIXES.md and extract automatic fixes
2. Group fixes by category
3. For each category in sequence:
   a. Group fixes by independent files/modules
   b. For each group (in parallel):
      - Dispatch subagent with specific file fixes
      - Subagent applies fixes, runs local verification (fmt, clippy for Rust)
   c. Wait for all subagents to complete
   d. Run full test suite:
      - Frontend: `npm run test`
      - Backend: `CARGO_BUILD_JOBS=2 cargo test --workspace`
   e. If tests pass: commit category changes
   f. If tests fail: rollback category changes, flag for manual review
4. After all categories, run final test suite and commit
```

## Detailed Design

### Category Processing Order

1. **Error-handling** (4 fixes)
   - Rust: Replace `unwrap()` with proper error propagation in 4 files
   - TypeScript: Add error logging, extract error handling utility

2. **Duplication** (5 fixes)
   - Extract mock data factory for demo APIs
   - Create generic sync utility for tauri-sync
   - Extract shared interface for web API method signatures
   - Extract private method for duplicate error handling blocks
   - Create shared grid component for similar grid layouts

3. **Memory** (3 fixes)
   - Implement worker pool/reuse in three-d-canvas.svelte
   - Implement scoped containers in dependency_injection.ts
   - Reuse/cache Maps and Sets in demo files

4. **Naming** (2 fixes)
   - Standardize TypeScript interface naming (snake_case → camelCase)
   - Standardize parameter naming across API files

### File Grouping Strategy

Within each category, we'll group fixes by independent modules:

- **Error-handling**: Group by language (Rust vs TypeScript) since they don't overlap
- **Duplication**: Group by functional area (demo, sync, web API, components)
- **Memory**: Group by component (canvas, DI container, demo APIs)
- **Naming**: Single group since fixes are across many files but independent

### Subagent Instructions

Each subagent receives:

1. Exact file paths and line numbers
2. Specific fix instructions from the prioritized list
3. Verification steps (run cargo fmt/clippy for Rust, npm run check for TypeScript)
4. Status reporting requirements (DONE, DONE_WITH_CONCERNS, BLOCKED, NEEDS_CONTEXT)

### Test Strategy

After each category:

1. Frontend tests: `npm run test` (currently 34 tests passing)
2. Backend tests: `CARGO_BUILD_JOBS=2 cargo test --workspace`
3. Linting: `cargo clippy --workspace --all-targets` for Rust changes
4. Type checking: `npm run check` for TypeScript changes

If tests fail:

- Rollback category changes with `git reset --hard HEAD~1`
- Flag failing fixes for manual review
- Continue with next category

### Error Handling

- **Subagent BLOCKED**: Retry once with more context, then escalate to user
- **Subagent NEEDS_CONTEXT**: Provide missing information, re-dispatch
- **Merge conflicts**: Unlikely due to file grouping; if occur, process sequentially
- **Test failures**: Rollback category, flag fixes, continue with next category

## Files to Modify

Based on PRIORITIZED_FIXES.md automatic fixes list:

### Error-handling (4)

1. `src-tauri/src/lib.rs:165` - unwrap on mutex lock
2. `src-tauri/src/lib.rs:267-268` - unwrap on array access
3. `src-tauri/src/tauri_app_state.rs:56` - unwrap on mutex lock
4. `service/src/import_service.rs:158` - multiple unwraps on file ops
5. `db/src/db_context.rs:17` - unwrap on database check
6. `src/lib/api/web/request.ts:56-61` - silently ignore JSON parse errors
7. `src/lib/api/web/request.ts:101-106` - duplicate error handling code
8. `src/routes/+layout.svelte:118` - generic error message
9. `src/lib/components/view/model-img.svelte:34` - silent failure
10. `src/lib/components/view/three-d-canvas.svelte:108` - console.warn

### Duplication (5)

1. `src/lib/api/demo/{model,label,group}.ts` - mock data structures
2. `src/lib/api/tauri-sync/sync-*.ts` - sync algorithm patterns
3. `src/lib/api/web/{model,label,group}.ts` - duplicate API method signatures
4. `src/lib/api/web/request.ts` - three identical error handling blocks
5. `src/lib/components/view/{model-grid,group-grid}.svelte` - similar grid logic

### Memory (3)

1. `src/lib/components/view/three-d-canvas.svelte:35-40` - new Worker each load
2. `src/lib/api/dependency_injection.ts:16` - container holds singletons indefinitely
3. `src/lib/api/demo/group.ts:32` - new Map each call
4. `src/lib/api/demo/label.ts:26` - new Set each call

### Naming (2)

1. `src/lib/api/shared/group_api.ts:74-81` - mixed snake_case/camelCase interface
2. `src/lib/api/shared/model_api.ts:86-94` - mixed parameter naming

## Commit Strategy

One commit per category with conventional commits:

- `fix(error-handling): replace unwrap with proper error propagation`
- `fix(duplication): extract shared utilities and interfaces`
- `fix(memory): implement caching and disposal patterns`
- `fix(naming): standardize TypeScript naming conventions`

Final commit: `refactor: apply all automatic code quality fixes`

## Success Criteria

1. All 14 automatic fixes applied correctly
2. All existing tests pass after each category
3. No regressions introduced
4. Changes committed to `refactor/simplify` branch
5. Code follows project style guidelines (rust-style.md, frontend-style.md)

## Risks and Mitigations

**Risk:** Fix breaks existing behavior
**Mitigation:** Tests after each category, rollback on failure

**Risk:** Merge conflicts between parallel subagents
**Mitigation:** File grouping ensures no overlapping changes

**Risk:** Subagent misinterprets fix instructions
**Mitigation:** Clear instructions with exact file/line references, self-review step

**Risk:** Performance degradation from caching changes
**Mitigation:** Existing tests should catch performance regressions
