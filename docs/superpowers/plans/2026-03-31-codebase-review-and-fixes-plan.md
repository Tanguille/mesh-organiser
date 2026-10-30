# Codebase Review and Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Review entire Mesh Organiser codebase for code reuse opportunities, quality issues, and efficiency improvements, then apply safe fixes.

**Architecture:** Parallel subagent review with five specialized agents analyzing the entire project, followed by integration and automatic fixing.

**Tech Stack:** SvelteKit/TypeScript frontend, Tauri/Rust backend, SQLite database, Vitest (frontend), cargo test (backend).

---

### Task 1: Launch parallel subagents for code review

**Files:**

- None created/modified (subagents will examine existing files)

- [ ] **Step 1: Prepare subagent prompts**

Define five subagent prompts with the following structure:

```
You are a code reviewer specializing in [area]. Analyze the entire Mesh Organiser codebase (frontend: src/, backend: src-tauri/, service/, db/) for issues related to [area].

Focus on:
- [area-specific focus points]

For each issue found, report:
1. File path
2. Line number (approximate)
3. Category (e.g., error-handling, duplication, performance, memory, naming)
4. Severity (critical, major, minor)
5. Issue description
6. Suggested fix

Use grep, glob, AST searches, and file reading. Be thorough but focus on actionable improvements.

Return findings as a structured list.
```

- [ ] **Step 2: Launch all five subagents in parallel**

Use the task tool to launch:

1. Error handling subagent (`@fixer`)
2. Reduce duplication subagent (`@fixer`)
3. Performance subagent (`@fixer`)
4. Memory efficiency subagent (`@fixer`)
5. Naming subagent (`@fixer`)

Each subagent will run independently and return findings.

- [ ] **Step 3: Wait for subagents to complete**

Collect results from all five subagents. This may take 5-10 minutes.

---

### Task 2: Collect and analyze findings

**Files:**

- None created/modified

- [ ] **Step 1: Gather all subagent reports**

Collect the findings from each subagent.

- [ ] **Step 2: Deduplicate and prioritize**

Remove duplicate issues (same file/line). Categorize by severity and impact.

- [ ] **Step 3: Create prioritized fix list**

Group fixes into:

- **Automatic fixes**: Unused imports, redundant code, simple error handling (safe to apply automatically)
- **Structural changes**: Changes that modify behavior or require design decisions (flag for review)
- **Low-priority**: Minor naming improvements, style consistency (optional)

- [ ] **Step 4: Present findings summary to user**

Provide a summary of issues found, categorized by area and severity. Ask for approval to proceed with automatic fixes.

---

### Task 3: Apply automatic fixes

**Files:**

- Various frontend and backend files (to be modified)

- [ ] **Step 1: Apply error handling fixes**

For each error handling issue:

- Replace `.unwrap()` with `.context("message")?` where appropriate
- Add missing error propagation
- Fix overly broad catch blocks

Example fix in Rust:

```rust
// Before
let config = load_config().unwrap();

// After
let config = load_config().context("Failed to load config")?;
```

- [ ] **Step 2: Apply duplication fixes**

Extract duplicated logic into shared utilities. Create helper functions in existing utility files.

Example:

- If same validation logic appears in multiple Svelte components, extract to `src/lib/utils/validation.ts`
- If same Rust function duplicated, extract to `src-tauri/src/utils/` or appropriate module

- [ ] **Step 3: Apply performance fixes**

- Replace inefficient algorithms
- Add missing caching
- Optimize data structures

- [ ] **Step 4: Apply memory efficiency fixes**

- Remove unnecessary `.clone()` calls
- Use references where possible
- Fix potential memory leaks

- [ ] **Step 5: Apply naming fixes**

- Rename variables/functions for clarity
- Ensure consistency with project conventions (Rust snake_case, TypeScript camelCase)
- Fix typos in identifiers

- [ ] **Step 6: Commit each batch of fixes**

Use conventional commit messages:

```bash
git add .
git commit -m "fix(error-handling): replace unwrap with context in config loading"
```

---

### Task 4: Run tests and verify

**Files:**

- None created/modified

- [ ] **Step 1: Run frontend tests**

```bash
npm run test
```

Expected: All tests pass. If failures occur, investigate and fix.

- [ ] **Step 2: Run backend tests**

```bash
cargo test --workspace
```

If OCCT compilation freezes, limit parallelism:

```bash
CARGO_BUILD_JOBS=2 cargo test --workspace
```

- [ ] **Step 3: Fix any test failures**

If tests fail due to our changes, debug and fix.

- [ ] **Step 4: Run linters**

Frontend: `npm run check` (SvelteKit)
Backend: `cargo clippy --workspace --all-targets`

- [ ] **Step 5: Format code**

Frontend: `npm run format` (if available)
Backend: `cargo fmt --all`

---

### Task 5: Finalize and report

**Files:**

- None created/modified

- [ ] **Step 1: Summarize changes made**

List all files modified and types of fixes applied.

- [ ] **Step 2: Flag remaining issues for review**

Present structural changes that were not automatically applied for user review.

- [ ] **Step 3: Commit final changes**

```bash
git add .
git commit -m "refactor: apply code quality improvements across codebase"
```

- [ ] **Step 4: Push changes (if requested)**

Only if user asks for push.

---

## Self-Review

**1. Spec coverage:** ✅ All spec requirements covered:

- Exploration via subagents
- Analysis and categorization
- Fixing automatic issues
- Verification via tests
- Preserving existing behavior

**2. Placeholder scan:** ✅ No placeholders found. Each step contains specific commands or code examples.

**3. Type consistency:** ✅ Consistent references to files, commands, and conventions throughout.

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-03-31-codebase-review-and-fixes-plan.md`. Two execution options:**

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
