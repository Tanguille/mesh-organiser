# Codebase Simplification Report

_Generated 2026-05-28 by a whole-codebase `/simplify` pass (review → apply safe → report the rest)._

## Scope & approach

A whole-codebase quality review (reuse / simplification / efficiency / altitude — **not** a correctness/bug hunt) was run across both stacks: the Rust crates (`db`, `service`, `web`, `libmeshthumbnail`, `src-tauri`) and the SvelteKit frontend (`src/`), excluding the generated `src/lib/components/ui/` primitives and lockfiles. The review produced **154 findings**, each tagged with **risk** (likelihood of behaviour change) and **confidence**.

Per the agreed delivery model:

- **Applied now:** only **low-risk + high-confidence** findings (dead code, clear duplication → shared helper, obvious redundancy), each conforming to `docs/rust-style.md` / `docs/frontend-style.md`, gated on a green build/lint/test.
- **Reported below:** everything riskier (medium/high risk or lower confidence) — for your review and approval.

## What was applied

| Category | Detail |
|---|---|
| Toolchain + deps | Workspace `rust-version` → **1.96.0**; `cargo update` (12 patch bumps); pnpm lockfile update (`eslint-plugin-svelte` 3.17→3.18 + transitives) |
| Dead code | Deleted `src-tauri/src/web_server.rs` (90 lines, unreferenced, imported absent `actix_*` deps) |
| Safe dedup/cleanup | **86** within-module findings across 9 buckets (Rust CRUD/row-mapping/validation helpers; frontend API-backend, sync, and component duplication) |
| Cross-cutting helpers | Frontend: `uniqueById`, `triggerDownload`/`triggerBlobDownload`, `representativeModel`, `getThisLabelOnly`, `redirectAfterUserSwitch`, shared `SizeOptionClasses`, new `app-header.svelte`. Rust: `ImportState::all_model_ids()` (collapsed 3 copies) |
| Lint cleanup | Fixed 6 clippy regressions in new helpers → **clippy clean**; 2 nightly-clippy `useless_borrows_in_formatting` fixes |
| Style conformance | 8 fixes vs the project style guides (full-word identifiers, blank-line-before-return) on top of `cargo fmt` + prettier |

**Net effect:** ~104 files changed, roughly **+1230 / −1625 lines (≈ −390 net)** — the tree is smaller and de-duplicated.

### Verification (final state)

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | ✅ clean |
| `cargo clippy --workspace --all-targets` (stable) | ✅ 0 warnings |
| `cargo +nightly clippy --workspace --all-targets` | ✅ 0 warnings (after the 2 fixes) |
| `cargo build --workspace --all-targets` | ✅ |
| `cargo test --workspace` | ✅ all pass (the one pre-existing `get_groups` failure was root-caused and fixed — see below) |
| `pnpm check` (svelte-check) | ✅ 0 errors / 0 warnings |
| `pnpm lint` (eslint) | ✅ clean |
| `prettier --check` (changed files) | ✅ clean |
| `pnpm test` (vitest) | ✅ 26/26 |

## ✅ Pre-existing failing test — root-caused and fixed

`db/tests/sql_queries_integration.rs::get_groups_filtered_by_ungrouped_model_do_not_expand_to_all_models` was failing in the pre-change baseline (so a latent pre-existing bug, not a regression from this work). Systematic debugging found **two distinct pagination defects**, both rooted in `GroupFilterOptions`/`ModelFilterOptions` deriving `Default` with `page = 0, page_size = 0` (an invalid pagination state the test reaches via `..Default::default()`):

1. **Panic:** `let offset = ((options.page - 1) * page_size) as usize;` — `0u32 - 1` underflows in debug builds.
2. **Silent empty result:** the in-memory `…skip(offset).take(page_size)` with `page_size == 0` is `.take(0)` → 0 groups regardless of what was built. (The grouping logic itself is correct — `convert_model_list_to_groups` builds the ungrouped virtual group with `id = -model.id`.)

**Fixes applied** (verified: `db` integration suite 7/7, full workspace 134 tests, 0 failures):

- `group_db.rs` pagination now uses `options.page.saturating_sub(1)` (guards an explicit `page = 0`).
- `GroupFilterOptions` and `ModelFilterOptions` now have a hand-written `Default` of `page = 1, page_size = MAX_PAGE_SIZE` (the codebase's "fetch all" convention) instead of the invalid `0/0`. Blast radius is zero — every production/internal call site sets `page`/`page_size` explicitly; only the test relied on `Default`.

---

## Round 2 — High-severity items: viability + implementation

Each of the 7 high-severity items below was first analysed for **behaviour-preservation viability** (read the real code, hunt for the kind of subtlety where "identical-looking" blocks actually differ). **6 of 7 were then implemented** in their behaviour-preserving form, with regression tests, and verified green (`cargo build`/`clippy`/`test`, vitest 45/45, svelte-check, eslint, prettier). Independent Codex review of the prior round reported **no correctness regressions**.

| # | Item | Verdict | Applied? | Scope / deferral reason |
|---|------|---------|----------|--------------------------|
| 1 | JoinSet driver | viable (partial) | ✅ `import`+`export` → `run_bounded` (service/util.rs) | `thumbnail_service` **excluded** — uses `spawn_blocking` + emits incremental per-task progress; folding it would batch the progress UI |
| 2 | Per-OS slicer | **deferred** | ⛔ | The 3 `open()` impls genuinely differ (empty-check location, `println` ordering vs `get_slicer_path().unwrap()`, error types); not a pure dedup, and macOS/Windows can't be compiled on the Linux dev host. Needs a deliberate behaviour-normalisation decision + platform CI |
| 3 | `StoredConfiguration`→serde | viable (Option A) | ✅ `#[serde(default)]` container **+ `slicer` override → None** | A naive container default would flip a missing `slicer` key from `None`→auto-detect (running `flatpak` probes at deserialize). Option A preserves `None`. Both call sites (`src-tauri/lib.rs`, `web/app.rs`) migrated; 3 test fixtures rewritten. *(Option B — align a missing key with the missing-**file** auto-detect, arguably a bugfix — is available on request; not done under a pure-dedup banner.)* |
| 4 | Zip-entry helper | viable (partial) | ✅ `find_zip_entry_bytes` for stl/obj/gcode/3mf-thumb | `step.rs` (streams to a temp file + reopens a path for OpenCascade) and `extract_image/gcode.rs` (streams via `BufReader::lines()`) **excluded** — forcing a `Vec<u8>` would change streaming/memory behaviour |
| 5 | Sync dispatch runner | viable (3 of 4) | ✅ `applySyncResult` for groups/labels/resources | `sync-models` **kept inline** — it has an extra pre-step **and two interleaved steps between `toDownload` and `syncToServer`** (ordering-sensitive) |
| 6 | Demo group loops | viable | ✅ `collectGroupModels` (demo/group.ts) | — |
| 7 | Raw-parsers → `shared/` | viable (mechanical) | ✅ moved to `shared/raw_model.ts`, thin re-exports kept in `tauri/` | parsers have zero tauri dependency; other backends unchanged |

**Only item #2 (slicer) remains unimplemented**, plus the deliberately-excluded sub-parts noted above (thumbnail loop, `step`/`extract_image-gcode` zip streaming, `sync-models` dispatch). The full per-item findings are retained below for reference.

---

## Reported findings (original review — for reference)

The findings are listed below, **highest value first**. Each notes its location, the cost of leaving it, and a concrete suggested fix.

### High severity — architectural dedup (status: see Round 2 table above) (7)

#### Bounded-parallelism JoinSet driver loop is copy-pasted across import, export, and thumbnail services with only the per-item closure and result handling differing

- **Location:** `service/src/import_service.rs` (lines 392-431)
- **Angle:** altitude · **Severity:** high · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Three near-identical 25-40 line loops (import_models_from_dir, export_to_temp_folder, thumbnail_service process loop) each re-implement: active counter, `if active >= max && join_next`, panic-resume on is_panic(), and final join_all. Any fix to the concurrency/panic logic must be applied in 3 places.
- **Suggested fix:** Add one generic helper (e.g. in service/src/util.rs) `run_bounded<T>(items, max, |item| async {...}) -> Vec<T>` that owns the JoinSet, active-count throttling, and is_panic()/resume_unwind handling, then have all three call sites pass only their closure. export collects PathBufs, import discards Ok, thumbnail increments a counter — these fit a `Vec<results>` return that callers post-process.

#### Three per-OS impl Slicer blocks duplicate the same open()/is_installed() control flow; only the locate+spawn step actually differs per platform.

- **Location:** `service/src/slicer_service/linux.rs, macos.rs, win.rs` (lines linux 7-65; macos 7-50; win 13-54)
- **Angle:** altitude · **Severity:** high · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Each of the three files re-implements: the Custom delegation, the is_installed() short-circuit, the !is_installed() error, the empty-paths error, and the println. Any change to that shared flow (e.g. fixing the empty-paths check, changing the not-installed message, adding logging) must be made in three places and is already drifting (macos uses `paths.len() == 0`, `if let Slicer::Custom`, no #[must_use]/#[errors] docs; the other two use `paths.is_empty()`, `matches!`, full docs).
- **Suggested fix:** Put the shared open()/is_installed() bodies once in base.rs (or a small platform trait). Define a per-OS-cfg private fn pair: `fn locate(slicer:&Slicer)->Option<PathBuf>` (or installed check) and `fn spawn_slicer(slicer:&Slicer, paths)`. base.rs's open() does Custom delegation + installed check + empty check + println, then calls the cfg-gated spawn_slicer. linux's flatpak path, macos's `open -a`, win's open_with_paths become the only per-OS code.

#### The entire StoredConfiguration struct + 124-line stored_to_configuration mapping exist only to fill missing JSON fields from defaults, which serde does natively.

- **Location:** `service/src/configuration.rs` (lines 9-55, 108-234)
- **Angle:** simplification · **Severity:** high · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Two parallel 45-field structs and a 124-line hand-written `.unwrap_or(default.x)` mapping (already annotated #[allow(too_many_lines)]) must be kept in lockstep with every new config field — three edit sites per field (StoredConfiguration, Configuration, the mapping, plus the two huge test fixtures). This is exactly what serde's container-level `#[serde(default)]` does: missing fields fall back to Configuration::default().
- **Suggested fix:** Add `#[serde(default)]` to the `Configuration` struct (it already derives Deserialize and has a Default impl), deserialize JSON straight into `Configuration` in src-tauri/src/lib.rs:506, and delete StoredConfiguration + stored_to_configuration. Verify the one nuance: `slicer` currently maps straight through (no default) — under serde(default) a missing `slicer` becomes Default's auto-detected installed slicer, which is the more correct behavior anyway.

#### The zip-archive iterate-and-find-entry-by-extension loop is copy-pasted across 6 format handlers with only the extension predicate and per-entry action varying.

- **Location:** `libmeshthumbnail/src/parse_model/stl.rs` (lines 41-64 (also obj.rs 44-67, gcode.rs 50-71, step.rs 62-95, extract_image/gcode.rs 51-68, extract_image/threemf.rs 23-45))
- **Angle:** altitude · **Severity:** high · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Six near-identical loops (open File, ZipArchive::new, for i in 0..zip.len(), by_index, Path::new(file.name()).extension().eq_ignore_ascii_case(...), read, then the same 'not found in zip' InternalError). Any fix (e.g. case-insensitive name match, error wording, handling nested dirs) must be edited in six places and they already drift (stl uses io::copy+Cursor, obj uses read_to_end, gcode buffers then Cursor, threemf matches by full name suffix).
- **Suggested fix:** Add a crate-internal helper, e.g. fn find_zip_entry<R: Read+Seek>(path: &Path, matches: impl Fn(&str) -> bool, not_found: &str) -> Result<Vec<u8>, MeshThumbnailError> that opens the zip, finds the first matching entry, returns its bytes (with_capacity(file.size())), and emits the InternalError on miss. Each format handler then calls it with its extension predicate and parses the returned buffer.

#### The final 'apply syncState' dispatch block (6 if-blocks: toUpload/toDownload/syncToServer/syncToLocal/toDeleteServer/toDeleteLocal) is duplicated near-identically across all four sync-*.ts files

- **Location:** `src/lib/api/tauri-sync/sync-resources.ts` (lines 155-201)
- **Angle:** altitude · **Severity:** high · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** The same ~45-line orchestration shape (compute differences, then conditionally run upload/download/syncToServer/syncToLocal/deleteServer/deleteLocal with the same local/remote arg swapping) is copy-pasted in sync-models.ts (282-317), sync-groups.ts (204-246), sync-labels.ts (224-276), and sync-resources.ts (155-201). Any change to the sync dispatch order or the toDownload/toDeleteLocal handling must be replicated in 4 places, and they already drift (models adds an in-progress reconciliation step the others lack).
- **Suggested fix:** Introduce a generic 'applySyncResult<T>(syncState, handlers)' in algorithm.ts (or a new sync-runner.ts) taking a handler object { upload, download, syncToServer, syncToLocal, deleteServer, deleteLocal } and running the six conditional steps. Each sync-*.ts then just builds the handlers; sync-models.ts keeps its extra in-progress reconciliation as a pre-step before calling the generic runner.

#### The grouped-models loop (56-93) and ungrouped-models loop (120-152) are near-identical copy-paste: same model_ids filter, same label_ids filter, same text_search predicate, and same Printed/Favorite flag accumulation.

- **Location:** `src/lib/api/demo/group.ts` (lines 56-152)
- **Angle:** simplification · **Severity:** high · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** ~75 lines of duplicated filter+flag logic; any change to the filter rules (e.g. a new flag or a search field) must be made in two places and is easy to drift.
- **Suggested fix:** Extract a private helper `collectGroupModels(modelIds, labelIds, textSearch, predicate)` that walks mockModels applying the three filters and returns {models, labelIds:Set, flags:string[]}. Call it once with `modelGroupMap.get(id) === groupId` and once with `!modelGroupMap.has(id)` to eliminate the second copy.

#### Raw-JSON parse functions (parseRawModel/parseRawGroup/parseRawLabel/etc.) live in tauri/ but are the shared server wire contract consumed by web, web_share and tauri-online backends.

- **Location:** `src/lib/api/tauri/model.ts` (lines 17-31)
- **Angle:** altitude · **Severity:** high · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** web/model.ts, web_share/model.ts, web/group.ts, web/resource.ts, web/threemf.ts, web_share/group.ts, web/label.ts all import parseRawModel/parseRawGroup/parseRawGroupMeta/parseRawLabel/parseRawLabelMeta/parseRawResourceMeta/RawGroup/RawLabel from ../tauri/*. The HTTP backends are structurally coupled to the 'tauri' backend folder for code that is not tauri-specific (it parses the identical JSON shape returned over both Tauri IPC and HTTP). Adding a new non-tauri backend forces it to reach into tauri/.
- **Existing helper:** `src/lib/api/shared/raw_model.ts (already hosts RawModel + parseRawModel-adjacent types; the parsers belong alongside it)`
- **Suggested fix:** Move parseRawModel/parseRawBlob/parseRawGroup/parseRawGroupMeta/parseRawLabel/parseRawLabelMeta/parseRawResourceMeta/convertResourceFlagsToRaw and the Raw* interfaces (RawGroup, RawLabel, RawLabelKeyword, RawResourceMeta) into shared/ (e.g. extend shared/raw_model.ts or new shared/raw_parsers.ts). Have tauri/*, web/*, web_share/* import the parsers from shared/. Tauri's *.ts can keep thin re-exports if other call sites depend on them.

### Medium severity (17)

#### Manual ModelFlags->string[] conversion (`if (model.flags.printed) flagsArray.push("Printed")` ...) reimplements the existing shared converter.

- **Location:** `src/lib/api/demo/group.ts` (lines 161-163)
- **Angle:** reuse · **Severity:** medium · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Duplicates flag-name string literals already centralised in shared/raw_model.ts; if a flag is renamed/added the demo silently diverges from the rest of the app.
- **Existing helper:** `src/lib/api/shared/raw_model.ts:convertModelFlagsToRaw`
- **Suggested fix:** Use `convertModelFlagsToRaw(model.flags) ?? []` from ../shared/raw_model. Note its null-return-when-empty semantics: wrap with `?? []` since createGroupInstance expects a string[].

#### The set_last_updated_on_X / set_last_updated_on_Xs single+plural pairs are near-identical per entity, differing only in table/column names.

- **Location:** `db/src/label_db.rs` (lines 508-545; group_db.rs:434-471; resource_db.rs:239-255)
- **Angle:** altitude · **Severity:** medium · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** Six functions (label single+plural, group single+plural, resource single) that all do `UPDATE <table> SET <col>_last_modified = ? WHERE <id_col> IN (...) AND <user_col> = ?`. Each new entity duplicates the whole pair; the plural variant's empty-guard + QueryBuilder + push_in_i64 boilerplate is repeated verbatim.
- **Suggested fix:** Add a private generic helper in query_util.rs, e.g. `async fn set_timestamp_column(db, table, ts_col, id_col, user_col, ids: &[i64], user_id, timestamp)` that builds the UPDATE...IN query once; have each set_last_updated_on_Xs delegate to it, and have the singular variants call the plural with a one-element slice (or a shared single-id helper). This deepens the mechanism instead of duplicating per table.

#### extract_metadata and extract_models hand-roll temp-dir creation instead of using export_service::get_temp_dir

- **Location:** `service/src/threemf_service.rs` (lines 203-204, 318-322)
- **Angle:** reuse · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Two more copies of the std::env::temp_dir().join("meshorganiser_..._action_{nanos}") + create_dir pattern that get_temp_dir already encapsulates; the action-name/timestamp format is now defined in 3 places and can drift.
- **Existing helper:** `service/src/export_service.rs::get_temp_dir`
- **Suggested fix:** In extract_models replace the manual temp_dir block with `let mut temp_dir = export_service::get_temp_dir("extract");`. For extract_metadata (which intentionally reuses a fixed, non-timestamped dir) leave as-is OR add a sibling helper if a stable dir is required; at minimum extract_models should call get_temp_dir. Note get_temp_dir uses create_dir not create_dir_all — equivalent here since the parent (temp dir) always exists.

#### The single-file import branch in import_path_inner duplicates import_models_from_dir_inner almost verbatim

- **Location:** `service/src/import_service.rs` (lines 163-192, 316-364)
- **Angle:** simplification · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Both blocks: read extension, read size via metadata, File::open, compute permanent_disk_path from import_as_path, call import_single_model with the same 8 args, add_model_id_to_current_set, then fs::remove_file if delete_after_import. Two copies of the same per-file import sequence to keep in sync.
- **Existing helper:** `service/src/import_service.rs::import_models_from_dir_inner`
- **Suggested fix:** Have the single-file branch in import_path_inner call import_models_from_dir_inner (passing path_buff, the Arc<Mutex<ImportState>>, user, origin_url, delete_after_import, import_as_path) instead of re-implementing the body. Requires reading user/origin_url/flags from the locked state first, matching how import_models_from_dir does it.

#### "fetch models by ids, collect &blob refs, generate_thumbnails" sequence duplicated in model and threemf controllers

- **Location:** `web/src/controller/model_controller.rs` (lines 436-446 (and threemf_controller.rs:101-111))
- **Angle:** reuse · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Both post::add_model and post::extract_threemf_models run the identical tail: `get_models_via_ids(...); let blobs: Vec<&Blob> = models.iter().map(|m| &m.blob).collect(); thumbnail_service::generate_thumbnails(&blobs, &app_state.app_state, false, &mut import_state).await?;`. Two copies of the blob-collect + thumbnail call drift independently.
- **Existing helper:** `service::thumbnail_service::generate_thumbnails`
- **Suggested fix:** Add a helper (e.g. in web_import_state.rs or a controller shared module) `async fn generate_thumbnails_for_models(app_state: &WebAppState, user: &User, model_ids: &[i64], import_state: &mut ImportState) -> Result<Vec<Model>, ApplicationError>` that does the get_models_via_ids + blob collect + generate_thumbnails, and call it from both handlers.

#### extract_threemf_models duplicates web/src/controller/threemf_controller.rs::extract_threemf_models, but uses brittle unwrap() where the web copy was hardened.

- **Location:** `src-tauri/src/lib.rs` (lines 187-225)
- **Angle:** altitude · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Same flow (extract_models -> collect ids -> get_models -> generate_thumbnails -> build ModelGroupMeta) implemented twice across crates. The lib.rs copy uses `import_state.imported_models[0].group_id.unwrap()` / `.group_name.clone().unwrap()`, whereas the web copy (threemf_controller.rs) already replaced this with `.first().and_then(|m| m.group_id.zip(m.group_name.clone())).ok_or_else(...)`. The shared logic has diverged, so the hardening fix has to be maintained twice.
- **Suggested fix:** Lift the shared body into a function in service (e.g. service::threemf_service::extract_threemf_group(model, user, app_state) -> ModelGroupMeta) that both the Tauri command and the web controller call, using the safer first()/zip()/ok_or_else form instead of indexing+unwrap.

#### Single-instance argv handling and the setup() argv handling are two near-identical deep-link/account-link parse blocks.

- **Location:** `src-tauri/src/lib.rs` (lines 527-563)
- **Angle:** simplification · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Both the single_instance closure (lib.rs:531-550) and setup() (lib.rs:627-642) do `if argv.len() == 2 { extract_deep_link(...); extract_account_link_via_deep_link(...); ... }`. The two diverge (one emits events, one sets InitialState fields) but share the parse-the-single-arg logic, so a new link scheme must be added in two places.
- **Suggested fix:** Extract a small free fn `parse_launch_arg(arg: &str) -> (Option<String> deep_link, Option<AccountLinkEmit> account_link)` and call it from both blocks, keeping only the emit-vs-store difference at each site.

#### stepUploadToRemote re-fetches ALL remote labels (`remoteApi.getLabels(false)`) inside the per-label loop

- **Location:** `src/lib/api/tauri-sync/sync-labels.ts` (lines 54-59)
- **Angle:** efficiency · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** For every uploaded label, `await remoteApi.getLabels(false)` re-fetches the entire remote label set just to resolve child labels. With N labels to upload this is N full label fetches over the network; the same anti-pattern repeats in stepSyncToRemote (116). The remote label list only grows by the label just added, so refetching the whole list each iteration is wasted I/O.
- **Suggested fix:** Fetch the remote labels once before the loop and maintain/append the newly added label to that list, or pass the already-fetched serverLabels/localLabels into the step; resolve child labels against the in-memory list instead of refetching per iteration.

#### ModelStreamManager and GroupStreamManager (group_api.ts 205-255) are near-identical generator-wrapper classes; the wrapping logic could be a single shared generic base.

- **Location:** `src/lib/api/shared/model_api.ts` (lines 227-285)
- **Angle:** simplification · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Two copies of the same private generator field + generateGenerator() + setSearchText/setOrderBy (which both just re-create the generator) + fetch() pattern. Any change to the prefetch/reset semantics must be mirrored in both classes.
- **Suggested fix:** Extract a generic abstract base, e.g. abstract class GeneratorStreamManager<T> { protected generator; protected abstract makeGenerator(): AsyncGenerator<T[]>; setOrderBy/setSearchText call this.regenerate(); fetch() returns (await generator.next()).value ?? []; }, and have both managers implement only makeGenerator() plus their typed setters.

#### PredefinedModelStreamManager.fetch() re-filters and re-sorts the entire model list on every page request, only to return one page slice.

- **Location:** `src/lib/api/shared/model_api.ts` (lines 183-220)
- **Angle:** efficiency · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** For a list of N models read in pages of 50, fetch() runs the full O(N) filter and O(N log N) sort on each of the ~N/50 calls, an avoidable O(N^2 / 50) total. The filter/sort result is identical between pages until setSearchText/setOrderBy is called.
- **Suggested fix:** Cache the filtered+sorted array, invalidating it in setSearchText/setOrderBy (which already reset fetchIndex). Compute the sorted list lazily once and slice [fetchIndex, fetchIndex+pageSize] from the cache; also note Array.prototype.sort mutates this.models in place today, so sorting a cached copy fixes that side effect too.

#### getShare() fetches the full /shares list and linear-scans for a match instead of hitting the per-id endpoint; the code comment 'Lazy implementation, should be fixed later' confirms it.

- **Location:** `src/lib/api/web/share.ts` (lines 47-58)
- **Angle:** efficiency · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Fetches and parses every share the user owns on each single-share lookup; O(n) network payload and scan for an O(1) lookup. The backend already exposes GET /shares/{share_id} (web/src/controller/share_controller.rs line 32).
- **Existing helper:** `src/lib/api/web_share/share.ts LimitedWebShareApi.getShare`
- **Suggested fix:** Replace the getShares()+loop with a direct `const rawShare = await this.requestApi.request<RawShare>(`/shares/${shareId}`, HttpMethod.GET); return parseRawShare(rawShare);` (same call LimitedWebShareApi.getShare already makes). Note: medium risk because the per-id endpoint is registered outside the login_required layer and is not owner-scoped, so for an authenticated caller it could resolve shares the list endpoint would not return — verify that is acceptable before applying.

#### setFlagOnAllModels edits each model with a separate awaited modelApi.editModel call in a sequential loop

- **Location:** `src/lib/components/edit/multi-model.svelte` (lines 137-160)
- **Angle:** efficiency · **Severity:** medium · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Toggling printed/favorite on a multi-selection issues one awaited round-trip per model in series (the in-code TODO already flags it 'terribly inefficient'). For large selections this is N sequential I/O calls and blocks the toast until all complete.
- **Suggested fix:** Run the edits concurrently with `await Promise.all(affected_models.map((m) => modelApi.editModel($state.snapshot(m))))`, or add/use a bulk setter on IModelApi analogous to labelApi.addLabelToModels/removeLabelFromModels which already operate on a model array in one call.

#### LabelTree re-finds the label in sidebarState.labels on every render despite already receiving the label meta

- **Location:** `src/lib/components/app-sidebar.svelte` (lines 338-341)
- **Angle:** efficiency · **Severity:** medium · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** The snippet's own TODO (`This find isn't great`) flags it. Each LabelTree invocation does `sidebarState.labels.find((l) => l.meta.id === label.id)`, and the tree recurses per node, so rendering the label tree is O(nodes * labels) linear scans on every reactive update of sidebarState.
- **Suggested fix:** Pass the full label-with-children entry down into LabelTree (children already iterate `labelWithChildren.children` which are LabelMeta — instead carry the resolved entry), or build a `Map<id, entry>` once from sidebarState.labels and look up via the map in the snippet to make it O(1) per node.

#### The whole src-tauri/src/api/*.rs layer mirrors web/src/controller/*.rs almost 1:1, differing only in transport (Tauri command vs HTTP handler).

- **Location:** `src-tauri/src/api/model_api.rs` (lines 20-339)
- **Angle:** altitude · **Severity:** medium · **Risk if applied:** high · **Confidence:** medium
- **Cost today:** Every CRUD command (models, groups, labels, resources, users, blobs) exists twice: e.g. add_group/edit_group/get_group_count in group_api.rs vs group_controller.rs, add_label/edit_label/set_childs_on_label in label_api.rs vs label_controller.rs, all wrapping the same db/service calls with the same validation strings. A behavior change (e.g. the 'Invalid order_by value' message, or the delete-currently-logged-in-user guard) must be applied in both crates and is already drifting.
- **Suggested fix:** Promote the per-entity business logic into the service crate (e.g. service::group_service::add_group(db, user, name) returning the domain type), then have both the Tauri command and the Axum controller be thin transport adapters around those service functions. This deepens the shared mechanism instead of maintaining two parallel command layers. Large refactor — scope incrementally per entity.

#### TauriSidebarStateApi is a near-copy of shared DefaultSidebarStateApi, differing only by omitting the optional IShareApi branch and hardcoding shareCount: 0.

- **Location:** `src/lib/api/tauri/sidebar_state.ts` (lines 13-45)
- **Angle:** simplification · **Severity:** medium · **Risk if applied:** high · **Confidence:** medium
- **Cost today:** An entire ~33-line class duplicates the Promise.all sidebar aggregation already in shared/sidebar_state_api.ts; the two must be maintained in parallel for every sidebar field change.
- **Existing helper:** `src/lib/api/shared/sidebar_state_api.ts:DefaultSidebarStateApi`
- **Suggested fix:** Use DefaultSidebarStateApi in tauri/init.ts and delete TauriSidebarStateApi. Note this is behavior-changing: in the proxy-share path tauri/init.ts registers an IShareApi (line ~170), so DefaultSidebarStateApi would surface a real share count instead of the current hardcoded 0 — confirm that is desired before applying.

#### text_search is applied three times: once per-model while collecting grouped models, once per-model while collecting ungrouped models, then again on the fully-assembled groups at 184-196.

- **Location:** `src/lib/api/demo/group.ts` (lines 70-78, 132-140, 183-196)
- **Angle:** simplification · **Severity:** medium · **Risk if applied:** high · **Confidence:** high
- **Cost today:** Redundant repeated string lowercasing/matching and a confusing dual-stage filter; the per-model filters at 70-78 and 132-140 are fully subsumed by the group-level filter at 184-196.
- **Suggested fix:** Drop the inline text_search checks inside both collection loops (collect all models regardless of search) and rely solely on the single group-level text_search filter at 184-196. Risk is high only because empty-group handling interacts; verify the 184-198 stage still excludes the right groups.

#### switchUser(currentUser) is called twice around setSyncState with no state change between them

- **Location:** `src/lib/components/view/web-account-link-popup.svelte` (lines 59-65)
- **Angle:** simplification · **Severity:** medium · **Risk if applied:** high · **Confidence:** low
- **Cost today:** `await userSwitchApi.switchUser(currentUser)` runs on line 59 and again on line 65 with only setSyncState in between. If the second call is a copy-paste artifact it is a redundant async round-trip; if intentional (re-reading sync state) it needs a comment because it reads as dead repetition.
- **Suggested fix:** Confirm whether the post-setSyncState switchUser is needed to refresh sync state. If not, delete the second call (line 65). If it is, add a comment explaining why a re-switch is required after setSyncState.

### Low severity / minor (33)

#### get_model_ids_via_sha256s manually re-implements the `(?, ?, ...)` IN-list builder that push_in_i64 already provides, just for &str binds.

- **Location:** `db/src/model_db.rs` (lines 389-399)
- **Angle:** reuse · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Hand-rolled `push("(")` + `separated(", ")` + loop + `push(")")` duplicates the exact shape of query_util::push_in_i64; the two will drift and the IN-list construction now lives in two forms.
- **Existing helper:** `db/src/query_util.rs::push_in_i64`
- **Suggested fix:** Generalize query_util::push_in_i64 into a generic `push_in<T: Encode + Type<Sqlite>>(builder, values: &[T])` (or add a sibling `push_in_str`) in query_util.rs and call it here instead of the inline separated loop; keep push_in_i64 as a thin wrapper if needed for callers.

#### The whitespace-skip inner loop in parse_command_string is redundant given the empty-arg guard.

- **Location:** `service/src/slicer_service.rs` (lines 108-120)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** On whitespace the code already only pushes when `!current_arg.is_empty()`, so consecutive separators naturally produce no empty args; the explicit `while chars.peek() is space/tab { chars.next() }` inner loop is dead complexity (extra nesting + a peekable just for this).
- **Suggested fix:** Delete the inner consecutive-whitespace skip loop (lines 112-119); the outer `if !current_arg.is_empty()` push already handles runs of separators. Then the trailing `chars.peek()` requirement may also be removable.

#### extract_user_via_id_and_hash and extract_user_via_share_id share an identical get_user_by_id-then-Some tail

- **Location:** `web/src/controller/blob_controller.rs` (lines 66-94)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Both helpers end with the same `let Ok(Some(user)) = get_user_by_id(...).await else { return None }; Some(user)` shape; extract_user_via_share_id is also the same share-owner lookup duplicated in the cross-controller finding above. Minor, but the two local helpers could share the user-fetch step.
- **Suggested fix:** Once the shared `resolve_share_owner` helper exists (see share-owner finding), reduce extract_user_via_share_id to call it and `.ok().map(|(_, u)| u)`, removing the duplicated get_user_by_id/Some boilerplate.

#### resolve_path_under_base is a trivial single-call wrapper around canonical_path_under_base + Into

- **Location:** `web/src/path_safety.rs` (lines 80-88)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Extra indirection layer: resolve_path_under_base just does canonical_path_under_base(...).await.map_err(Into::into). canonical_path_under_base has no external callers (only this wrapper + tests), so the two functions express one operation split in two.
- **Suggested fix:** Either fold canonical_path_under_base's body into resolve_path_under_base (keeping UnderBaseError internal), or have callers use canonical_path_under_base + `?`/`Into` directly. Keep one public entry point with OpenUnderBaseError classification.

#### The f64-component -> Vec3<f32> conversion (Vec3::new(a.x as f32, a.y as f32, a.z as f32)) is hand-written in four places.

- **Location:** `libmeshthumbnail/src/parse_model/threemf.rs` (lines 33-42 (also step.rs 50-53, 160-162; obj.rs 87-92))
- **Angle:** reuse · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Same cast-each-component pattern repeated for 3mf vertices, step vertices, step STL vertices, and obj positions, each carrying its own #[allow(clippy::cast_possible_truncation)]. Drifts in style (struct literal vs Vec3::new).
- **Suggested fix:** Add one small helper fn vec3_f32_from(x: f64, y: f64, z: f64) -> Vec3<f32> (or a generic over the source point type) carrying the single cast allow, and call it from all four sites.

#### The string-array-to-flags forEach+switch decoder is reimplemented per type (resource_api ResourceFlags, model_api stringArrayToModelFlags, user_api createUserInstance permissions).

- **Location:** `src/lib/api/shared/resource_api.ts` (lines 28-34)
- **Angle:** altitude · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Three independent copies of 'iterate string[] and toggle booleans by matching a tag' (resource_api 28-34, model_api 16-25, user_api 34-46), plus the inverse encoders (permissionsToStringArray, convertModelFlagsToRaw). New flag types must re-handwrite the same loop.
- **Suggested fix:** Provide a generic pair, e.g. flagsFromTags<T>(tags: string[], map: Record<string, keyof T>) and tagsFromFlags<T>(flags, map), in a shared module, and define each type's map (e.g. { Printed: 'printed', Favorite: 'favorite' }). Lower priority since each map is tiny; only worth it if more flag types appear.

#### The two `groupFlags`/`ungroupedFlags` accumulators do the same ModelFlags->string[] mapping as convertModelFlagsToRaw, just guarded against duplicate pushes; a Set or the shared converter would express it directly.

- **Location:** `src/lib/api/demo/group.ts` (lines 86-91, 145-150)
- **Angle:** reuse · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Two more copies of the Printed/Favorite literal mapping (4 total in this file), each manually deduping via includes().
- **Existing helper:** `src/lib/api/shared/raw_model.ts:convertModelFlagsToRaw`
- **Suggested fix:** Collect flags into a `Set<string>` per group by spreading `convertModelFlagsToRaw(model.flags) ?? []`, then `Array.from(set)` at the end, removing the manual `!includes()` guards.

#### `selected.map(x => x.models).flat()` recomputed inline in many places instead of using one derived

- **Location:** `src/lib/components/view/group-grid.svelte` (lines 246-250)
- **Angle:** efficiency · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** The expression `selected.map((x) => x.models).flat()` is recomputed inline at group-grid.svelte:413, 416, 433, 503, 510, 530, 534 on top of the existing derivation logic, so the flatten runs repeatedly per render across DragSelectedModels, RightClickModels, ModelGridInner availableModels, and the multiselect-all branch.
- **Suggested fix:** Introduce one `const selectedModelsFlat = $derived(selected.flatMap(x => x.models))` and reference it everywhere the inline `.map().flat()` is used (note selectedModels at line 246 already special-cases splitViewSelectedModels and should keep its own logic).

#### Progress-indicator Card shell duplicated between sync and import indicators

- **Location:** `src/lib/components/view/tauri-import-progress-indicator.svelte` (lines 53-74)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Both sync-progress-indicator and tauri-import-progress-indicator hand-roll the same Card.Root + Card.Header (`expanded-text-parent @container flex flex-row items-center justify-center gap-2 px-1 py-2`) + LoaderCircle spinner + truncated status text scaffold. The shared chrome is copy-pasted with slight variation in the body.
- **Suggested fix:** Extract a small presentational component (e.g. ProgressIndicatorCard taking an icon snippet, a status string, and an optional content snippet) and have both indicators render it, so the Card/header/spinner layout lives in one place.

#### onValueChange branches on subset only to pick a cast; both branches do the same thing

- **Location:** `src/lib/components/view/sort-filter.svelte` (lines 61-68)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** The ternary on `restProps.subset === "groups"` selects between two casts of the same `$state.snapshot(value)` and calls onchange with it. At runtime both branches are identical work (the cast is erased); the branch exists purely to satisfy the discriminated-union typing and reads as redundant logic.
- **Suggested fix:** Collapse to a single `onValueChange={() => (onchange as OnChangeCallback<any>)($state.snapshot(value))}` (or a single cast through unknown), removing the duplicated ternary branches.

#### Two LinkButton instances with identical props differ only by an outer HoverCard wrapper

- **Location:** `src/lib/components/edit/model.svelte` (lines 291-314)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** The if/else renders the exact same `<LinkButton link={model.link} class="widthhack h-full" variant="ghost" withText={false} withFallback={true} />` in both branches; the only difference is the HoverCard wrapper in the truthy branch. Duplicated prop list must be kept in sync by hand.
- **Suggested fix:** Render a single LinkButton and conditionally wrap only the trigger, or factor the LinkButton into a snippet `{#snippet linkBtn()}...{/snippet}` and render `{@render linkBtn()}` inside HoverCard.Trigger in the link branch and bare in the else branch, so the prop list lives once.

#### `printed`/`favorited` derived booleans duplicate the same `models.every(x => x.flags.<flag>)` shape

- **Location:** `src/lib/components/edit/multi-model.svelte` (lines 57-59)
- **Angle:** altitude · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** Two near-identical derivations and two near-identical setters (setPrintedFlagOnAllModels/setFavoriteFlagOnAllModels) exist for what is a generic per-flag aggregate+set. Adding a third boolean flag would require copying the whole pattern again.
- **Suggested fix:** Keep the two CheckboxWithLabel rows but drive them from a single generic helper already half-present: setFlagOnAllModels takes an action+value, so the per-flag wrappers can be inlined at the call site as `(val) => setFlagOnAllModels((x) => (x.flags.printed = val), val)`. Removes setPrintedFlagOnAllModels/setFavoriteFlagOnAllModels indirection.

#### Pluralization ternary `count === 1 ? "" : "s"` is copy-pasted in countWriter and 4 times in timeSinceDate

- **Location:** `src/lib/utils.ts` (lines 38-40,145-160)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** low · **Confidence:** medium
- **Cost today:** The same `${n} word${n === 1 ? "" : "s"}` shape is duplicated five times within this one file (countWriter plus each branch of timeSinceDate). Minor copy-paste that a tiny `pluralize(n, word)` helper would collapse.
- **Suggested fix:** Add a small `pluralize(count: number, word: string): string` returning `${count} ${word}${count === 1 ? "" : "s"}` and have countWriter and each timeSinceDate branch call it (e.g. `${pluralize(days, "day")} ago`).

#### get_unique_id_from_X_id and get_unique_ids_from_X_ids are structurally identical across model/group/label/resource, differing only in table/column names.

- **Location:** `db/src/model_db.rs` (lines 324-357; group_db.rs:197-228; label_db.rs:189-229; resource_db.rs:161-175)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** Eight functions implementing the same two query shapes (single SELECT unique_global_id, and SELECT id,unique_global_id WHERE id IN (...) -> IndexMap). The plural empty-guard + QueryBuilder + push_in_i64 + row-loop is repeated four times.
- **Suggested fix:** Extract a generic query_util helper `async fn fetch_global_ids(db, table, id_col, gid_col, ids, user_col: Option<(&str,i64)>) -> Result<IndexMap<i64,String>>` and a single-id counterpart; have each *_db wrapper delegate. Note group_db has no user scoping while the others do, so the user filter must be optional.

#### delete_dead_groups runs one DELETE query per dead group inside a loop instead of a single batched DELETE.

- **Location:** `db/src/group_db.rs` (lines 353-374)
- **Angle:** efficiency · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** One round-trip per dead group; the dead-blob analogue (blob_db::get_and_delete_dead_blobs) already does a single batched `DELETE ... WHERE blob_id IN (...)` after collecting ids.
- **Existing helper:** `db/src/blob_db.rs::get_and_delete_dead_blobs (batched DELETE...IN pattern)`
- **Suggested fix:** Collect group_ids from the dead-group rows and issue one `DELETE FROM models_group WHERE group_id IN (...)` via push_in_i64 (the per-user delete_group call here ignores user scoping anyway since the rows already came from the table), mirroring get_and_delete_dead_blobs.

#### download_file builds its temp dir inline instead of reusing get_temp_dir

- **Location:** `service/src/download_file_service.rs` (lines 168-172)
- **Angle:** reuse · **Severity:** low · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** Fourth copy of the meshorganiser_<action>_action_<nanos> temp-dir convention; format drift risk and an extra create_dir_all vs the shared helper.
- **Existing helper:** `service/src/export_service.rs::get_temp_dir`
- **Suggested fix:** Replace the env::temp_dir().join(format!(...)) + fs::create_dir_all block with `let temp_dir = crate::export_service::get_temp_dir("download");`. (get_temp_dir panics on create failure rather than returning Err — acceptable, matches the other call sites.)

#### Share-scoped variants (get_share_models / get_share_groups) are special-cased copies of the authed list handlers

- **Location:** `web/src/controller/model_controller.rs` (lines 127-163 (get_models vs get_share_models))
- **Angle:** altitude · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** get_share_models and get_share_groups re-implement the authed list path: they resolve a user from the share, force-clear/intersect the id filters, and then call the same inner builder. group_controller goes further and copies the entire GroupFilterOptions construction (lines 159-184) rather than reusing get_groups's body, so the two filter blocks must be maintained in lockstep. The share path is a special case bolted on rather than the list mechanism being parameterized by an 'effective user + id restriction'.
- **Suggested fix:** Factor the GroupFilterOptions construction in group_controller into a single inner fn (mirroring model_controller's get_models_inner) parameterized by user, model_ids override, and the share-vs-auth flags (include_ungrouped default, allow_incomplete_groups, label scoping). Have both get_groups and get_share_groups call it so the filter is built in one place.

#### startImportProcess assigns the same five fields from importResult to both the global importState and localImportState with duplicated statements in the loop.

- **Location:** `src/lib/api/tauri/tauri_import.ts` (lines 107-117)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** Ten lines of paired assignments (origin_url/failure_reason/recursive/delete_after_import/imported_models pushed to two targets) that drift if a field is added; the two objects are kept manually in lockstep.
- **Suggested fix:** Factor the shared fields into a small applyResult(target, importResult) helper called for both importState and localImportState, or build the field set once and Object.assign it to both.

#### startImportProcess maintains importStateClone in parallel with the reactive importState, snapshotting before resetImportState then re-assigning the same imported_models/status to both at the end.

- **Location:** `src/lib/api/tauri-online/tauri_import.ts` (lines 89-142)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** The clone is captured pre-reset but every field it ultimately returns (imported_models, status) is overwritten at the end identically to importState, making the snapshot and the paired final assignments (137-142) redundant duplicate bookkeeping.
- **Suggested fix:** Drop importStateClone; set the fields on importState only and return importState (or a single defaultImportState seeded with importedModelsSet + Finished status), removing the duplicated final assignment pair.

#### Line 96 skips empty named groups unless include_ungrouped_models, but line 198 unconditionally drops every group with zero models, making the 96 guard partly dead.

- **Location:** `src/lib/api/demo/group.ts` (lines 96, 198)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** Two competing empty-group rules; the `!include_ungrouped_models` condition at 96 reads as if empty groups survive in ungrouped mode, but 198 removes them anyway, so the guard is misleading.
- **Suggested fix:** Remove the conditional skip at 96 (always `continue` on empty, or just let 198 handle it) so empty-group filtering lives in exactly one place.

#### The model sort switch (AddedAsc/Desc, NameAsc/Desc, SizeAsc/Desc, ModifiedAsc/Desc) duplicates the comparator already implemented in PredefinedModelStreamManager.fetch in shared/model_api.ts.

- **Location:** `src/lib/api/demo/model.ts` (lines 64-85)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** Two copies of the same ModelOrderBy comparator; adding an order-by case (the demo even adds Modified cases the shared one lacks) means keeping them in sync by hand.
- **Suggested fix:** Extract the ModelOrderBy comparator into an exported `compareModels(orderBy)` in shared/model_api.ts and call it from both the demo getModels sort and PredefinedModelStreamManager.fetch.

#### lastSync uses a 1s setInterval + manual $state instead of a derived ticker

- **Location:** `src/lib/components/view/sync-progress-indicator.svelte` (lines 13-26)
- **Angle:** efficiency · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** A setInterval recomputes updateLastSync() every second and writes $state even when currentUser.lastSync hasn't crossed a human-readable boundary, plus it requires the explicit onDestroy clearInterval bookkeeping. The recompute fires continuously regardless of whether the sync card is the active stage.
- **Suggested fix:** Keep the interval only as a tick source (a $state counter) and make lastSync a $derived of currentUser.lastSync + tick, or gate the interval to only run while globalSyncState.stage == SyncStage.Idle so it isn't recomputing during active sync when `progress` shows percentages instead.

#### web_share Model/Group APIs declare 'implements ModelApi'/'implements GroupApi' (the tauri concrete classes) instead of the shared IModelApi/IGroupApi interfaces.

- **Location:** `src/lib/api/web_share/model.ts` (lines 10)
- **Angle:** altitude · **Severity:** low · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** WebShareModelApi implements ModelApi (web_share/model.ts:10) and WebShareGroupApi implements GroupApi (web_share/group.ts:10), importing the concrete tauri classes purely for their type. This couples web_share to tauri's class identity rather than the shared interface contract, and is inconsistent with every other backend which implements IModelApi/IGroupApi. If tauri's class signature changes for tauri-only reasons, web_share breaks for no real reason.
- **Existing helper:** `src/lib/api/shared/model_api.ts: IModelApi; src/lib/api/shared/group_api.ts: IGroupApi`
- **Suggested fix:** Change web_share/model.ts to 'implements IModelApi' (import from shared/model_api) and web_share/group.ts to 'implements IGroupApi' (import from shared/group_api), matching web/* and demo/*.

#### Demo backend re-implements the ModelOrderBy/GroupOrderBy sort comparators that already exist in shared/model_api.ts and shared/group_api.ts.

- **Location:** `src/lib/api/demo/model.ts` (lines 64-85)
- **Angle:** reuse · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** DemoModelApi.getModels (demo/model.ts:64-85) duplicates the same ModelOrderBy switch found in PredefinedModelStreamManager.fetch (shared/model_api.ts:198-214); DemoGroupApi.getGroups (demo/group.ts:201-218) duplicates the GroupOrderBy switch in PredefinedGroupStreamManager.fetch (shared/group_api.ts:189-200). Sort logic for the same enums lives in two places (the demo versions even add the Modified* cases the shared ones omit), so ordering can drift between demo and the shared managers.
- **Existing helper:** `src/lib/api/shared/model_api.ts: PredefinedModelStreamManager.fetch sort; src/lib/api/shared/group_api.ts: PredefinedGroupStreamManager.fetch sort`
- **Suggested fix:** Extract pure comparator factories, e.g. modelOrderByComparator(orderBy): (a,b)=>number and groupOrderByComparator(orderBy) in shared/model_api.ts / shared/group_api.ts, use them inside the Predefined*StreamManager and call the same factory from the demo getModels/getGroups sorts (adding the Modified* cases once in the shared factory).

#### LimitedWebShareApi and WebShareApi both implement IShareApi with near-identical no-op/throw stubs and a shared getShare/parseRawShare path; the limited variant largely restates the full one minus a few methods.

- **Location:** `src/lib/api/web_share/share.ts` (lines 9-43)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** web_share/share.ts (LimitedWebShareApi) reimplements getShares (returns []), getShare (request + parseRawShare, already importing from ../web/share), and stubs getShareLink/createShare/addModelsToShare/setModelsOnShare/editShare/deleteShare. Most of this is the same surface as web/share.ts WebShareApi; the only real difference is which methods are no-ops in a read-only share context. Two share API classes must be kept in sync as IShareApi evolves.
- **Existing helper:** `src/lib/api/web/share.ts: WebShareApi, parseRawShare`
- **Suggested fix:** Have LimitedWebShareApi extend WebShareApi (or a shared base) and override only the methods that must be disabled in the share viewer, instead of restating getShare/parseRawShare and every stub; reuse the existing parseRawShare/getShare from web/share.ts.

#### Tauri commands call state.get_current_user() repeatedly within one handler; each call locks a Mutex and clones the whole User

- **Location:** `src-tauri/src/api/label_api.rs` (lines throughout (16 calls); model_api.rs (10), group_api.rs (8), resource_api.rs (9), user_api.rs (4))
- **Angle:** efficiency · **Severity:** low · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** get_current_user (tauri_app_state.rs:55) does `current_user.lock().unwrap().clone()` - a full User clone (8 String/Option<String> fields) under a mutex - and handlers like edit_model, delete_models, set_labels_on_model, set_label_on_models, set_childs_on_label call it 2-3x each. ~47 calls across the 5 api files, many redundant within a single command.
- **Existing helper:** `src-tauri/src/tauri_app_state.rs::get_current_user`
- **Suggested fix:** Bind `let user = state.get_current_user();` once at the top of each command and reuse `&user` (the pattern resource_api.rs::remove_resource already follows at line 84). Removes redundant lock+clone per extra call.

#### Every axum handler opens with the identical `let user = auth_session.user.unwrap().to_user();` line - 39 copies across the 5 controllers

- **Location:** `web/src/controller/model_controller.rs` (lines every handler (7 calls); group_controller (7), label_controller (12), resource_controller (6), user_controller (7))
- **Angle:** reuse · **Severity:** low · **Risk if applied:** medium · **Confidence:** high
- **Cost today:** 39 repetitions of the same unwrap-and-convert. Beyond verbosity, the bare .unwrap() is copy-pasted into every handler; a single extractor would centralize it and make the conversion uniform.
- **Existing helper:** `web/src/user.rs::to_user (the conversion is already a single method; the extractor would call it)`
- **Suggested fix:** Introduce an axum extractor (e.g. `struct CurrentUser(db::model::user::User)` implementing FromRequestParts that pulls AuthSession and calls to_user once), then take `CurrentUser(user)` as a handler argument. Replaces the repeated boilerplate line in all 39 handlers with a typed parameter.

#### add_<entity> commands re-synthesize the *Meta response struct client-side (random_hex_32 + time_now for unique_global_id/created/last_modified) instead of returning what the db actually wrote

- **Location:** `src-tauri/src/api/group_api.rs` (lines add_group 66-86; label_api.rs add_label 30-51; resource_api.rs add_resource 24-45; and controller counterparts group_controller post 311-331, label_controller post 109-133, resource_controller post 84-104)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** medium · **Confidence:** medium
- **Cost today:** 6 copies (3 tauri + 3 controller) of 'call add_<entity> for the id, then build a *Meta with a fresh random_hex_32() and time_now()'. The synthesized hex/timestamp are a second random/clock read that does NOT match the values add_<entity> persisted (add_empty_group/add_label/add_resource each generate their own hex+now internally), so the response is a fabricated parallel copy.
- **Existing helper:** `db/src/util.rs::random_hex_32, db/src/util.rs::time_now (currently called twice - once in db, once again in each caller)`
- **Suggested fix:** Have the db add_<entity> functions return the populated *Meta (or the generated unique_global_id + created timestamp) so callers return the persisted values directly, eliminating the duplicated meta-construction in all 6 handlers and the value-mismatch. This pushes the fix down to the db layer (altitude) instead of reconstructing in every caller.

#### get_model_count and import_path_inner repeat the same dir/recursive/zip/supported dispatch ladder with the same zip-extension and import_as_path checks

- **Location:** `service/src/import_service.rs` (lines 97-127, 138-197)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** high · **Confidence:** medium
- **Cost today:** The is_dir -> recursive?, extension == zip (+ import_as_path zip guard), is_supported_extension, else Unsupported branch structure is written twice; the `is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))` test and the 'Cannot import a zip as path' guard are each duplicated.
- **Suggested fix:** Extract a small `enum ImportKind { Dir, Zip, File }` classifier (taking path + recursive + import_as_path) that both functions match on, centralizing the zip-extension test and the zip-as-path error. Higher risk because both functions return errors in slightly different orders, so verify the zip-as-path guard ordering is preserved.

#### convert_extension_to_zip/convert_zip_to_extension re-encode the stl/obj/step/gcode<->zip mapping that db's FileType already owns.

- **Location:** `service/src/util.rs` (lines 96-119)
- **Angle:** reuse · **Severity:** low · **Risk if applied:** high · **Confidence:** low
- **Cost today:** The canonical zippable-extension set and its .zip suffixing is duplicated here and in db (FileType::to_zip/from_zip + to_extension). Adding a new zippable format means editing both db and these util functions.
- **Existing helper:** `db/src/model/blob.rs::FileType::to_zip / from_zip / to_extension`
- **Suggested fix:** Consider routing through db: `FileType::from_extension(ext).to_zip().to_extension()` and `.from_zip().to_extension()`. NOTE: behavior differs on unknown input (util passes through the lowercased original string; FileType maps unknown to Unknown which panics in to_extension), so this is not a drop-in — only worth it if callers are guaranteed-known extensions. Flagged low-confidence for that reason.

#### Manual byte-to-hex loop reimplements hex encoding for the generated local password

- **Location:** `web/src/app.rs` (lines 125-134)
- **Angle:** reuse · **Severity:** low · **Risk if applied:** high · **Confidence:** low
- **Cost today:** The `for b in key_bytes { write!(pass, "{b:02X}") }` loop hand-rolls hex encoding. db::util uses hex::encode (db/src/util.rs:7) for the same byte->hex job. NOTE: the loop emits UPPERCASE; hex::encode emits lowercase, and the `hex` crate is currently a db dep, not a web dep, so this is not a drop-in swap and would change the generated password format.
- **Existing helper:** `db/src/util.rs random_hex_32 (uses hex::encode)`
- **Suggested fix:** Only if the uppercase requirement is incidental: add hex to web/Cargo.toml and replace the loop with `hex::encode_upper(key.master())`. If password format/case must stay stable, leave as-is.

#### Labels and resources upload/sync/delete steps run strictly sequentially while groups and models use runGeneratorWithLimit for bounded concurrency

- **Location:** `src/lib/api/tauri-sync/sync-labels.ts` (lines 30-65)
- **Angle:** efficiency · **Severity:** low · **Risk if applied:** high · **Confidence:** medium
- **Cost today:** sync-groups.ts and sync-models.ts process items with `runGeneratorWithLimit(..., 4)` (4 concurrent), but sync-labels.ts (stepUploadToRemote, stepSyncToRemote, deleteFromRemote) and sync-resources.ts (all three steps) use plain `for ... await`, serializing every network round-trip. For large label/resource sets this is materially slower for no structural reason.
- **Suggested fix:** Wrap the per-item bodies of the label/resource steps in the same `runGeneratorWithLimit` concurrency pattern used by groups/models. Risk is high only because label child-ordering (sortFunction) and the shared globalSyncState counter assume sequential progress — verify ordering constraints before parallelizing, or parallelize only the independent resource step.

#### ModelStreamManager.getAll() is a stub returning [] while PredefinedModelStreamManager.getAll() returns the real list; the consumer in model-grid.svelte relies on it for select-all.

- **Location:** `src/lib/api/shared/model_api.ts` (lines 282-284)
- **Angle:** simplification · **Severity:** low · **Risk if applied:** high · **Confidence:** medium
- **Cost today:** getAll() on the IModelStreamManager interface silently returns an empty array for the live (non-predefined) stream, so model-grid.svelte:142 sets allModels to [] for paged streams. Either getAll is unimplemented (latent gap) or the method does not belong on the streaming manager at all.
- **Suggested fix:** Either implement getAll() by draining the generator / calling getModels with a large page, or remove getAll() from IModelStreamManager and have the single consumer (model-grid.svelte) handle the predefined-vs-paged distinction explicitly. Flagged as risk:high because either direction changes select-all behavior.

