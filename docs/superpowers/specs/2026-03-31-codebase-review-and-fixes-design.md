# Codebase Review and Fixes Design

**Date**: 2026-03-31  
**Topic**: Review entire Mesh Organiser codebase for code reuse, quality, and efficiency improvements, then apply fixes.

## Purpose

Identify and fix code quality issues, reduce duplication, improve performance, memory efficiency, and naming consistency across the entire project (frontend SvelteKit/TypeScript, backend Tauri/Rust, database). Apply automatic fixes for obvious issues and flag structural changes for review.

## Scope

- **Entire project**: frontend (`src/`), backend (`src-tauri/`, `service/`, `db/`), configuration files.
- **Focus areas**: error handling, duplication, performance, memory efficiency, naming.
- **Preserve existing behavior** — only simplify, don't refactor.

## Approach

Parallel subagent review: launch five specialized subagents simultaneously, each analyzing the entire codebase from their perspective.

### Subagent Assignments

1. **Error handling** (`@fixer` with error handling focus)
   - Missing error propagation
   - Missing handlers (unwrap/expect without context, unhandled async errors)
   - Inconsistent error types
   - Overly broad catch blocks

2. **Reduce duplication** (`@fixer` with DRY focus)
   - Duplicated logic across files/functions
   - Opportunities to extract shared utilities
   - Redundant conditionals or branches

3. **Performance** (`@fixer` with performance focus)
   - Unnecessary allocations or clones
   - Inefficient data structures
   - Redundant computations
   - Missing caching/memoization

4. **Memory efficiency** (`@fixer` with memory focus)
   - Clones, allocations, leaks
   - Unnecessary owned vs borrowed data
   - Memory-intensive patterns

5. **Naming** (`@fixer` with naming focus)
   - Clarity, consistency, conventions
   - Typos, ambiguous names
   - Rust naming conventions, TypeScript naming conventions

## Process

### Phase 1: Exploration (parallel)

Each subagent explores the codebase using:

- `grep` for patterns (e.g., `.unwrap()`, `clone()`, duplicated code)
- `glob` for file discovery
- AST searches for structural issues
- Reading key files to understand context

### Phase 2: Analysis (parallel)

Each subagent categorizes issues by severity:

- **Critical**: Bugs, memory leaks, undefined behavior
- **Major**: Missing error handling, significant duplication, performance bottlenecks
- **Minor**: Naming inconsistencies, small duplication, minor inefficiencies

Each subagent reports findings in a structured format:

```
File: path/to/file.rs
Line: 42
Category: error-handling
Severity: major
Issue: `unwrap()` on Result without context
Suggestion: Replace with `.context("meaningful message")?`
```

### Phase 3: Integration

Orchestrator (main agent) collects all findings, deduplicates, and prioritizes.

### Phase 4: Fixing

- **Automatic fixes**: Apply obvious improvements (unused imports, redundant code, simple error handling).
- **Flag structural changes**: Changes that modify behavior or require design decisions → present for review.

### Phase 5: Verification

After fixes, run tests:

- Frontend: `npm run test` (Vitest)
- Backend: `cargo test --workspace` or `cargo test -p service`
- Ensure no regressions.

## Constraints

- Preserve existing behavior — only simplify, don't refactor.
- Fix obvious issues automatically (unused imports, redundant code).
- Flag structural changes for review before applying.
- Follow project coding standards (Rust style, Svelte patterns).
- Use existing patterns and utilities.

## Success Criteria

- All identified issues are addressed or flagged.
- No regressions in functionality.
- Tests pass after fixes.
- Code is cleaner, more maintainable, and more efficient.

## Risks

- Over-automation could break functionality.
- Subagents may overlap or conflict.
- Performance changes may have unintended side effects.

## Mitigation

- Conservative approach: only apply safe, obvious fixes.
- Test after each batch of changes.
- Manual review for structural changes.

## Timeline

- Exploration & analysis: ~10 minutes (parallel)
- Integration & fixing: ~20 minutes
- Verification: ~5 minutes

Total: ~35 minutes.
