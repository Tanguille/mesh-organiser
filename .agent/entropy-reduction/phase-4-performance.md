# Phase 4: Performance Analysis
**Completed:** 2026-03-07

## Findings

### Database Layer (Rust/sqlx)

#### 1. N+1 Query Patterns
- **label_db.rs:208-231** - `add_labels_on_models` uses nested loops with database queries inside
  - Outer loop: `for label_id in label_ids`
  - Inner loop: `for model_id in model_ids` with individual INSERT queries
  - **Impact:** O(n*m) queries for n labels and m models
  
- **group_db.rs:345-365** - `delete_dead_groups` fetches dead groups, then loops to delete each
  - Queries each dead group individually instead of batching

- **label_db.rs:411-419** - `add_childs_to_label` loops and executes individual INSERTs
- **label_db.rs:442-450** - `remove_childs_from_label` loops with individual DELETEs

#### 2. Unbounded Queries
- **label_db.rs:16-34** - `get_labels_min` fetches ALL labels without user filter or pagination
  - No LIMIT clause: `.fetch_all(db)`
  - Called from `model_db.rs:126` during every model query

- **model_db.rs:189** - `get_models_via_ids` uses `page_size: u32::MAX`
  - Can return unlimited results when fetching models by IDs

- **group_db.rs:97-180** - `get_groups` fetches ALL models first (`page_size: u32::MAX`)
  - Processes entire dataset in memory before pagination
  - Line 117: `page_size: u32::MAX` defeats purpose of pagination
  - Called twice when filtering (lines 108-120 and 140-150)

- **thumbnail_service.rs:109** - `generate_all_thumbnails` fetches ALL blobs
  - `blob_db::get_blobs(&app_state.db).await` - no filtering or pagination

#### 3. Missing Database Indexes
The following frequently queried columns lack indexes:

| Table | Column | Usage | Migration Fix |
|-------|--------|-------|---------------|
| blobs | blob_sha256 | SHA256 lookups during import | `CREATE INDEX idx_blobs_sha256 ON blobs(blob_sha256)` |
| models | model_group_id | Group filtering, joins | `CREATE INDEX idx_models_group_id ON models(model_group_id)` |
| models | model_blob_id | Blob joins | `CREATE INDEX idx_models_blob_id ON models(model_blob_id)` |
| models | model_unique_global_id | Global ID lookups | `CREATE INDEX idx_models_global_id ON models(model_unique_global_id)` |
| models_group | group_unique_global_id | Group lookups | `CREATE INDEX idx_groups_global_id ON models_group(group_unique_global_id)` |
| labels | label_unique_global_id | Label lookups | `CREATE INDEX idx_labels_global_id ON labels(label_unique_global_id)` |
| label_keywords | keyword_label_id | Keyword joins | `CREATE INDEX idx_keywords_label_id ON label_keywords(keyword_label_id)` |
| label_keywords | keyword_name | Keyword search | `CREATE INDEX idx_keywords_name ON label_keywords(keyword_name)` |
| shares_models | share_id | Share lookup | `CREATE INDEX idx_shares_models_share_id ON shares_models(share_id)` |
| shares_models | model_id | Model share lookup | `CREATE INDEX idx_shares_models_model_id ON shares_models(model_id)` |

### Frontend Layer (Svelte 5)

#### 4. $effect Usage Patterns
The following components use `$effect` which should be reviewed for potential infinite loops:

- **model-grid.svelte:79-81** - Sets orderBy based on configuration (safe, no circular deps)
- **model-grid.svelte:116-124** - Resets model set when stream changes (uses `untrack`, safe)
- **group-grid.svelte:123-125** - Sets group orderBy (safe)
- **group-grid.svelte:129-135** - Filters selected based on loaded groups (safe)
- **group-grid.svelte:304-308** - Clears split view models when selection changes (safe)
- **group-grid.svelte:310-317** - Resets group set when stream changes (safe)
- **three-d-canvas.svelte:116-128** - Loads model when props change (uses `untrack` + id check, safe)
- **model.svelte:67-69** - Sets editMode from initialEditMode (safe)
- **model.svelte:87-115** - Fetches 3MF metadata (uses `untrack`, safe)
- **label.svelte:83-90** - Refreshes keywords when label changes (safe with id check)
- **group.svelte:41-43** - Sets editMode from initialEditMode (safe)

**Verdict:** All `$effect` usages appear safe with proper guards (`untrack`, id checks). No infinite loop risks detected.

#### 5. Large Lists Without Virtualization
- **group-grid.svelte:434-454** - Renders ALL loaded groups in DOM
  - No virtualization for large group lists
  - `{#each loadedGroups as group}` renders entire collection
  - Same issue in List view (lines 434-442) and Grid view (lines 444-454)

- **model-grid-inner.svelte** - Likely has similar issue (not reviewed in detail)

#### 6. Missing Resource Cleanup
- **+layout.svelte:50** - `addEventListener("unhandledrejection", ...)` without corresponding `removeEventListener`
  - Event listener leaks on component destroy

- **group-grid.svelte:127-139** - `setInterval(handleScroll, 1000)` properly cleaned in `onDestroy`
  - Good example of proper cleanup

#### 7. Worker Management
- **three-d-canvas.svelte:29-77** - Creates Worker on each model load
  - Worker terminated after use (line 41: `worker.terminate()`)
  - Proper cleanup pattern

### Service Layer (Rust)

#### 8. Sync Operations in Async Context
- **export_service.rs:267-305** - `get_size_of_blobs` uses blocking file I/O
  - `std::fs::read_dir()` in async function without `spawn_blocking`
  - Lines 275-302 iterate directory synchronously

- **import_service.rs:249-283** - `import_models_from_dir_recursive` uses blocking I/O
  - `std::fs::read_dir()` at line 254
  - Recursively traverses directories synchronously

- **import_service.rs:559-579** - `get_model_count_from_dir_recursive` blocking I/O

#### 9. File Handle Management
Generally good practices observed:
- **service/src/export_service.rs:173, 177** - Proper `stream_writer.close()` and `writer.close()`
- **service/src/import_service.rs:540** - Proper `writer.close()`
- **src-tauri/src/api/web_extensions_api.rs:118, 123** - Proper close calls

Potential issues:
- **patch/libmeshthumbnail** - Multiple `File::open()` calls without explicit close
  - Relies on Rust's Drop trait (acceptable but worth noting)

#### 10. Large Payloads / Over-fetching
- **model_db.rs:64-72** - Model query joins 4 tables and uses GROUP_CONCAT
  - Returns all label data embedded in each model row
  - For models with many labels, GROUP_CONCAT can hit SQLite limits
  
- **group_db.rs:97-180** - Groups endpoint loads ALL models into memory
  - Double query pattern when filtering loads dataset twice
  - No streaming or chunked processing

- **label_db.rs:61-84** - Label query has 3 correlated subqueries
  - `parent_label_model_count`, `parent_label_group_count`, `parent_label_ungrouped_count`
  - Executes subquery for EACH parent label row (N+1 pattern)

## Performance Issues

| Severity | Location | Issue | Impact | Fix |
|----------|----------|-------|--------|-----|
| Critical | db/src/label_db.rs:61-84 | Correlated subqueries causing N+1 | O(n) subqueries per label fetch | Pre-calculate counts or use JOIN with GROUP BY |
| Critical | db/src/group_db.rs:97-180 | Loads ALL models (u32::MAX) | Memory exhaustion with large libraries | Implement proper pagination at DB level |
| High | db/src/label_db.rs:208-231 | Nested loops with DB queries | O(n*m) query count | Batch insert with single query |
| High | db/src/model_db.rs:189 | Unlimited page_size | Can fetch entire DB into memory | Add reasonable max limit |
| High | service/src/export_service.rs:267-305 | Blocking file I/O in async | Blocks tokio runtime | Use `spawn_blocking` |
| High | service/src/import_service.rs:249-283 | Blocking directory traversal | Blocks tokio runtime | Use `spawn_blocking` or async-fs |
| Medium | db/src/label_db.rs:16-34 | Unbounded label fetch | Memory pressure | Add LIMIT or pagination |
| Medium | db/src/thumbnail_service.rs:109 | Fetches ALL blobs | Memory pressure with many blobs | Add batching/chunking |
| Medium | Multiple migrations | Missing indexes | Slow queries on large datasets | Add indexes per table above |
| Medium | src/routes/+layout.svelte:50 | Event listener leak | Memory leak on navigation | Add `onDestroy` with `removeEventListener` |
| Low | src/lib/components/view/group-grid.svelte | No list virtualization | DOM bloat with many groups | Implement virtual list |
| Low | db/src/group_db.rs:43-93 | "Insanely inefficient" comment | CPU overhead converting models | Documented TODO, consider refactoring |

## Action Items

- [ ] **CRITICAL:** Refactor `label_db.rs:61-84` to eliminate correlated subqueries - either pre-calculate counts in separate queries or use JOIN with GROUP BY pattern
- [ ] **CRITICAL:** Fix `group_db.rs` pagination to use LIMIT/OFFSET at database level instead of fetching all models
- [ ] **HIGH:** Batch insert in `add_labels_on_models` - build single INSERT with multiple VALUES
- [ ] **HIGH:** Add `spawn_blocking` wrapper around blocking file I/O in export_service.rs and import_service.rs
- [ ] **HIGH:** Add reasonable maximum page_size limit (e.g., 1000) to prevent `u32::MAX` queries
- [ ] **MEDIUM:** Add missing database indexes (see table above for complete list)
- [ ] **MEDIUM:** Implement batch processing for thumbnail generation
- [ ] **MEDIUM:** Add `removeEventListener` cleanup in +layout.svelte
- [ ] **LOW:** Implement virtual scrolling for large group/model lists
- [ ] **LOW:** Profile `convert_model_list_to_groups` and optimize if needed

## Summary Stats
- Total issues: 12
- Critical: 2 | High: 4 | Medium: 5 | Low: 2

### Risk Areas by Component
1. **Database Layer (db/src/)** - 7 issues: N+1 patterns, unbounded queries, missing indexes
2. **Service Layer (service/src/)** - 3 issues: Blocking I/O, batch operations
3. **Frontend (src/lib/components/)** - 2 issues: List virtualization, event cleanup
