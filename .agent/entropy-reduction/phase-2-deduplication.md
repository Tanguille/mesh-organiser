# Phase 2: Code Deduplication
**Completed:** 2026-03-07

## Findings

### Overview
The Mesh Organiser codebase exhibits significant duplication patterns across the API abstraction layer, Rust service implementations, error handling, and data transformation logic. The most critical duplications involve:

1. **Triplicate API Implementations**: Every API has 3 implementations (Tauri, Web, Demo) with nearly identical structures
2. **Dual Import Services**: Two nearly identical `import_service.rs` files (service/src/ and src-tauri/src/service/)
3. **Duplicate Error Enums**: `ApplicationError` and `ServiceError` share 80%+ structure
4. **Copy-Paste Parse Patterns**: Raw data parsing logic repeated across all API files

### Critical Findings

#### 1. Import Service Duplication (CRITICAL)
Two files with ~90% identical logic:
- `service/src/import_service.rs` (668 lines) - Async/tokio version
- `src-tauri/src/service/import_service.rs` (458 lines) - Sync/rayon version

Both implement the same 15+ functions with identical algorithms but different async/sync patterns.

#### 2. Triplicate API Layer (CRITICAL)
Every entity API has 3 implementations:
- **Tauri**: `src/lib/api/tauri/*.ts` - Uses `invoke()` 
- **Web**: `src/lib/api/web/*.ts` - Uses `fetch/HTTP`
- **Demo**: `src/lib/api/demo/*.ts` - Uses in-memory mock data

Files affected: 55+ API implementation files

#### 3. Error Enum Duplication (HIGH)
`ApplicationError` (src-tauri/src/error.rs) and `ServiceError` (service/src/service_error.rs) share:
- Identical variant structure (FileSystemFault, InternalError, JsonError, DatabaseError)
- Identical `Serialize` implementation pattern
- 8/10 variants are conceptually the same

#### 4. Parse/Transform Pattern Duplication (HIGH)
Every Tauri API file contains identical patterns:
```typescript
export interface RawXXX { ... }
export function parseRawXXX(raw: RawXXX): XXX { ... }
```

This pattern appears 16+ times across API files.

#### 5. Sync Algorithm Duplication (HIGH)
Four sync files with nearly identical logic:
- `sync-models.ts` (188 lines)
- `sync-groups.ts` (119 lines)  
- `sync-labels.ts` (86 lines)
- `sync-resources.ts` (163 lines)

All use the same `computeDifferences`, `stepUpload`, `finalizeSync` patterns.

### Medium Priority Findings

#### 6. Database Query Pattern Duplication (MEDIUM)
Repeated patterns in `db/src/*.rs`:
- 87 occurrences of `sqlx::query!`
- Repeated transaction patterns
- Identical error mapping: `.map_err(|e| ApplicationError::InternalError(e.to_string()))`

#### 7. Invoke/Fetch Wrapper Duplication (MEDIUM)
All Tauri API methods follow identical pattern:
```typescript
async methodName(params): Promise<ReturnType> {
  return await invoke<RawType>("command_name", { params });
}
```

41 occurrences across 17 files.

#### 8. Component Structure Duplication (LOW-MEDIUM)
Edit components (`edit/model.svelte`, `edit/group.svelte`, `edit/label.svelte`) share:
- Identical card/layout structure
- Similar edit mode toggle patterns
- Repeated API dependency injection patterns

## Duplications Found

| Files | Pattern | Lines Affected | Suggested Abstraction |
|-------|---------|----------------|------------------------|
| `service/src/import_service.rs` + `src-tauri/src/service/import_service.rs` | Duplicate import logic (sync vs async) | ~600 lines each | Create `import_core.rs` with shared algorithms, use adapter pattern for I/O |
| `src/lib/api/tauri/*.ts` (17 files) | `invoke()` wrapper pattern | ~400 lines | Generic `TauriApiClient` class with method builders |
| `src/lib/api/web/*.ts` (17 files) | `fetch()` wrapper pattern | ~350 lines | Generic `WebApiClient` class with method builders |
| `src/lib/api/demo/*.ts` (12 files) | Mock data CRUD operations | ~500 lines | Generic `MockApiBase` class with common CRUD |
| `src-tauri/src/error.rs` + `service/src/service_error.rs` | Error enum definitions | 75 + 81 lines | Shared `error_types.rs` in common crate or trait-based error abstraction |
| `src/lib/api/tauri/*` + `src/lib/api/web/*` | `parseRaw*` functions | ~200 lines | Codegen from OpenAPI spec or shared deserialization utilities |
| `src/lib/api/tauri-sync/sync-*.ts` (4 files) | Sync algorithm steps | ~500 lines total | Generic `SyncEngine<T>` with configurable sync strategies |
| `src/lib/api/shared/*_api.ts` | API interface definitions + stream helpers | ~800 lines | Consolidate streaming logic into generic `StreamManager<T>` |
| `db/src/*_db.rs` (10 files) | `sqlx::query!` patterns | ~600 lines | Query builder macros or repository trait pattern |
| `src/lib/components/edit/*.svelte` (3 files) | Edit card structure | ~150 lines each | Reusable `EditCard` component with slots |
| `src/routes/*/+page.svelte` | Page layout + data loading | ~100 lines each | Generic `DataPage` component or layout inheritance |
| `src-tauri/src/api/*_api.rs` | Result<_, ApplicationError> return types | ~200 lines | Type alias `AppResult<T> = Result<T, ApplicationError>` |

## Action Items

### Immediate (Critical)
- [ ] **Consolidate Import Services**: Merge `import_service.rs` implementations into a single async version with optional sync adapter
- [ ] **Create API Base Classes**: Build generic base classes for Tauri/Web/Demo APIs to eliminate wrapper duplication
- [ ] **Unify Error Types**: Create shared error types between src-tauri and service crates

### Short Term (High Priority)
- [ ] **Extract Parse Utilities**: Move all `parseRaw*` functions to shared deserialization utilities
- [ ] **Generic Sync Engine**: Refactor sync-* files to use generic `SyncEngine<T>`
- [ ] **Repository Pattern**: Implement repository traits for database layer to standardize queries

### Medium Term
- [ ] **Component Abstraction**: Create reusable `EditCard`, `DataGrid` components
- [ ] **API Codegen**: Generate API clients from OpenAPI spec to eliminate manual API implementations
- [ ] **Type Aliases**: Add `AppResult<T>` type alias throughout Rust codebase

### Long Term
- [ ] **Unify API Implementations**: Consider if Demo API can wrap Tauri API with mock backend
- [ ] **Macro for Query Patterns**: Create macros for common SQLx query patterns
- [ ] **Unified Service Layer**: Merge src-tauri/service and service/ into single abstraction

## Summary Stats
- **Total issues:** 12 major duplication patterns identified
- **Critical:** 3 | **High:** 4 | **Medium:** 3 | **Low:** 2
- **Lines of duplicated code:** ~3,500+ lines (estimated 15-20% of total codebase)
- **Files affected:** 55+ files

## Risk Assessment
- **High Risk**: Import service consolidation requires careful testing of import functionality
- **Medium Risk**: API refactoring may affect type inference in dependent components
- **Low Risk**: Error type unification is mostly additive change

## Recommendations

1. **Start with Error Types**: Unifying `ApplicationError` and `ServiceError` is low-risk and establishes patterns for other refactors
2. **Extract Core Import Logic**: Move shared algorithms to `import_core.rs` first, then adapt I/O
3. **Generic API Base**: Create `BaseTauriApi` and `BaseWebApi` classes with method builder pattern
4. **Incremental Sync Refactor**: Port one sync type at a time to `SyncEngine<T>`
5. **Test Coverage**: Ensure high test coverage on import/sync before major refactoring
