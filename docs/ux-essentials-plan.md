# Elementary UX Implementation Plan

> **For agentic workers:** Use `superpowers:executing-plans` to implement this plan task by task. A separate test agent owns tests for each surface, as required by AGENTS.md. Update the checkboxes and evidence after each step.

**Goal:** Make everyday model selection, deletion, organization, navigation, and recovery work consistently with mouse and keyboard.

**Architecture:** Extend the existing selection helper, model/group editors, custom internal drag system, and injected APIs. Reuse soft deletion, labels, groups, and the existing group listing API. Do not add dependencies, a parallel folder hierarchy, or a generic command framework.

**Tech stack:** Svelte 5, TypeScript, bits-ui components, Vitest and Testing Library; existing Tauri/web/demo API implementations.

**Request:** “We are missing many crucial elementary UX features. I can't use the delete button to delete models for example, I can't drag things into folders etc, let astra do a thorough sweep and let it create a plan to fix this which you execute.”

## Branch and worktree

- Audit performed on `main`, commit `3bf3505a`, in `/home/tanguille/Documents/PriveProjecten/mesh-organiser`.
- Implementation branch: `feat/ux-essentials`, in `/home/tanguille/Documents/PriveProjecten/mesh-organiser` (the audit checkout, which had no unrelated changes).
- Before every work step, verify `pwd`, `git branch --show-current`, `git worktree list`, and `git status --short`.
- The lead agent completed `git pull` and `git status` before this document was written. Repeat before making changes in a newly selected checkout.
- When creating a worktree, copy `.mcp.json`, `.env`, `Codex.local.md`, `.vscode/`, and `.Codex/` when present; do not print secrets.
- Do not push or make a draft PR ready. Neither action is requested.

## Process Instructions

- After completing each step, update the plan with the current status.
- Pause for user confirmation before proceeding to next step.
- Suggest the prompt for continuing to the next step.
- After the last step, make a final documentation pass. Once the contents of the plan have been consolidated into existing documentation, the plan file can be removed. If there is no relevant existing documentation, the plan should be reworked into a reference document.

The user subsequently instructed “Keep implementing all steps”, authorizing continuous execution without pausing between steps for confirmation.

## Scope and behavior decisions

- “Delete” means **Move to trash**, using `IModelApi.deleteModels`; never permanent file deletion. Recovery already exists on `/trash`.
- Existing **groups** are exclusive model membership; existing hierarchical **labels** are additive tags. Support drops onto both, with copy that says “Move to group” or “Add label.” Neither operation moves source files on disk.
- The audit cannot establish whether “folders” meant groups, sidebar labels, or filesystem directories. Implement the first two existing concepts; filesystem move/export semantics require clarification and are not inferred.
- Keyboard shortcuts act only within the active model/group selection surface. Inputs, textareas, selects, contenteditable elements, open menus/dialogs, read-only shares, and repeated destructive keydown events must remain safe.
- Ctrl/Cmd+A selects **currently loaded results**, announced as such; do not silently fetch/select an entire library. Existing group-page “all models” behavior remains distinct and must obey active filters.
- Keep single-click preview. Enter and double-click open a real group. Space selects/toggles. F2 enters the existing name editor for one model or real group and focuses/selects its name. Escape cancels dragging first, then closes/clears the active selection.
- All destructive/move actions snapshot their target models before awaiting. UI reconciliation uses those ids even if selection changes during the request.
- Apply visible mutations after API success, keep failed items selectable, and report action-specific failure. A sidebar-refresh failure must not make a successful model mutation appear undone.

## Audit findings

These are code-confirmed findings, not claims of completed desktop/browser interaction tests. Paths refer to the audited commit.

| Priority | Confirmed gap                                                | Evidence and user impact                                                                                                                                                                                                                                                                                   |
| -------- | ------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1       | Delete key and ordinary collection shortcuts are absent      | `src/lib/utils.ts:245` handles only Enter/Space; `view/grid-selection.svelte.ts:125` delegates to it. No Delete, select-all, clear-selection, or arrow navigation exists in the model/group grids.                                                                                                         |
| P1       | Trash is unnecessarily hidden                                | `edit/model.svelte:290-326` exposes Move to trash only after enabling edit mode and opening ellipsis. `view/right-click-models.svelte` has no delete action. The backend already safely trashes/restores models (`service/src/export_service.rs:322`, shared model API).                                   |
| P1       | Existing internal drops are narrow and fragile               | `view/drag-selected-models-root.svelte:24-45` supports only labels and rejects SVG/path targets with `instanceof HTMLElement`. `view/drag-selected-models.svelte:43` stops dragging before the root sees a bubbling in-grid mouseup. No cancel-on-Escape/window blur and no destination highlight.         |
| P1       | No way to move one model to an arbitrary existing group      | `edit/model.svelte:186-206` offers only ungroup/new group. `edit/multi-model.svelte:54-59` derives destinations only from already-selected models' groups, excluding all other destinations. `IGroupApi.addModelsToGroup` and `getAllGroups` already exist.                                                |
| P1       | Search can leave hidden models selected                      | `view/model-grid.svelte:51-63` replaces loaded rows without clearing/pruning selection. Bulk actions can still operate on an invisible old selection.                                                                                                                                                      |
| P1       | Successful deletion can remove the wrong displayed selection | Editors await the API then call parameterless callbacks (`edit/model.svelte:147`, `edit/multi-model.svelte:191`); grid callbacks read the current selection (`view/model-grid.svelte:71`, `view/group-grid.svelte:158`). Switching selection while a request is pending produces incorrect reconciliation. |
| P1       | Deleted/filtered models can remain in bulk-action data       | `view/model-grid.svelte:84` removes only `loadedModels`; `allModels` remains stale. Search resets only loaded rows, while default bulk editor uses `allModelsWithFallback`. Predefined streams retain their backing arrays after local deletion and can reintroduce rows on another search/sort.           |
| P1       | A transient load failure can permanently stall the grid      | Both `view/model-grid.svelte:40-57` and `view/group-grid.svelte:72-89` set busy before awaiting and never reset it on rejection; reset then waits forever. There is no results loading/error/retry UI.                                                                                                     |
| P1       | Failed async actions stay disabled                           | `ui/button/async-button.svelte:12-15` restores active only on successful completion. Open/download/browser actions cannot be retried after failure.                                                                                                                                                        |
| P2       | Shift ranges stop working after the first range              | `view/grid-selection.svelte.ts:67` permits a range only when exactly one item is selected. There is no persistent range anchor. Every option is a tab stop and lacks an owning listbox role.                                                                                                               |
| P2       | Group navigation can show the previous group                 | `view/group-page.svelte:24` fetches only on mount even though the route updates its group id prop. A same-route parameter change does not necessarily remount the component. Ungrouping on `/group/[slug]` has no navigation callback.                                                                     |
| P2       | Group cards do not open through ordinary activation          | No group double-click handler exists; Enter only selects. “View models” is conditional on more than one member.                                                                                                                                                                                            |
| P2       | Visible controls lack names and capability consistency       | Grid checkboxes lack accessible names; model icon buttons/toggles lack labels; search has only placeholder text. Context menu always shows Open in folder/slicer even when their injected APIs are absent, so items can do nothing.                                                                        |
| P2       | No actionable empty-results state                            | Main model/group results render an empty region when no results exist; “No model/group selected” describes the detail pane, not whether the library is empty, filtered, loading, or failed.                                                                                                                |

Positive foundations: reusable grid selection, Ctrl/Cmd click and initial Shift range already exist; backend soft-trash and restore already exist; Tauri, web, and demo implement group assignment; current import page and trash page provide useful loading/error patterns; sidebar labels already carry drop target attributes. Native OS file import already listens to `tauri://drag-drop` in `api/tauri/tauri_import.ts` and must remain intact.

## Step 0 — Audit the existing workflows

**Status:** Complete: source audit and implementation plan only.

- [x] Verified the checkout path, branch, worktree list, clean starting status, and updated base commit `3bf3505a`.
- [x] Traced model/group selection, editors, context menus, custom drag source/root, sidebar targets, group navigation, trash recovery, and import handling.
- [x] Checked shared API contracts and Tauri/web/demo group implementations; checked the service soft-delete path and native drag-drop import listener.
- [x] Inspected existing grid, drag, import, and trash testing patterns and documented the missing regression cases.
- [x] Wrote prioritized implementation steps and explicit acceptance criteria. No product code was changed and no implementation tests were run during the audit.

**Evidence:** Source paths and audited behavior are listed in the findings table. The startup checkout was `main@f399451f`; the lead agent fast-forwarded it to `main@3bf3505a` during the audit, and the final branch/head were reverified before saving this plan. Product code touched by this audit was unchanged by that update.

**Continue prompt:** “Verify the branch/worktree and execute step 1 of docs/ux-essentials-plan.md.”

## Step 1 — Make collection state and async actions reliable

**Status:** Complete.

**Files:** `src/lib/components/view/model-grid.svelte`, `group-grid.svelte`, `group-page.svelte`; `src/lib/components/edit/model.svelte`, `multi-model.svelte`, `group.svelte`; `src/lib/components/ui/button/async-button.svelte`; affected callbacks in `src/routes/group/[slug]/+page.svelte` and `src/routes/import/+page.svelte`; `src/lib/api/shared/model_api.ts` only if the existing predefined stream needs a narrow way to remove members.

- [x] Dedicated test agent: explain stale-selection and failed-request regressions, list cases, then extend `view/grids.test.ts`, `edit/model.test.ts` and new focused tests for group-page and async-button. Keep one test surface per agent.
- [x] Capture current successful behavior and run the baseline; add failing tests for rejected fetch/action, changing selection while delete is pending, filtering with selection, deletion followed by search/sort, and group-id changes.
- [x] Release busy state in `finally`; use request generations so stale results cannot overwrite newer filters/routes. Provide a retry that regenerates a failed stream, since a rejected async generator is finished. Avoid repeatedly auto-retrying failures on the one-second scroll poll.
- [x] Change deletion/group-removal callbacks to carry the captured affected models/group. Reconcile loaded rows, bulk caches, selection, and predefined source membership using those ids. Do not hide newly selected models because a previous request completed.
- [x] Clear selection when filtering changes; ensure default bulk actions operate only on active results. Reset any editor-local deleted state when its model changes.
- [x] Make GroupPage react to the group id with stale-response protection; navigate to Groups after the current group is ungrouped. Keep import-page callbacks working.
- [x] Restore AsyncButton availability after both success and failure while preserving rejection reporting.
- [x] Verify tests and `pnpm run check`; record evidence and update status.

**Acceptance:** A failed fetch can be retried; typing another search never hangs. Deleting A while selecting B removes A only. A trashed item does not return after sort/search or remain in bulk operations. Editing B does not show A's “moved to trash” state. Navigating between two group ids displays the correct current group.

**Evidence:** Frontend checks passed: 373 tests across 52 files, Svelte check with zero diagnostics, oxlint, and oxfmt. Focused tests covered failed loads and retry, changed routes and late responses, deletion and selection races, partial ungrouping, predefined list recreation and paging, stale bulk results, and unloaded group members. New guards were observed failing before fixes or through temporary mutations; changes were restored. Svelte MCP autofixer was unavailable. Aikido scan could not authenticate because the environment has no accessible keychain or API key. Native desktop interaction was not exercised.

**Continue prompt:** “Verify the branch/worktree and execute step 2 of docs/ux-essentials-plan.md.”

## Step 2 — Expose safe trash actions consistently

**Status:** Implemented and covered by focused tests.

**Files:** `view/model-grid-inner.svelte`, `model-grid.svelte`, `group-grid.svelte`, `right-click-models.svelte`, `grid-selection.svelte.ts`; `edit/model.svelte`, `multi-model.svelte`; one small shared model-action helper only if needed to avoid duplicating API/error/busy logic across these callers.

- [x] Test agent extends grid/editor tests and adds context-menu integration cases: one/many selections, repeated Delete, editable target, read-only mode, pending/failing request, and split-grid sub-selection.
- [x] Add Delete handling to the active collection using the same trash action as the context menu and editor. Capture affected models once, prevent duplicate in-flight execution, and never use permanent deletion.
- [x] Add “Move to trash” to model context menus and an accessible action in the selection editor without requiring edit mode. Apply it only to explicitly selected models; keep group and ungroup actions separate.
- [x] Show success and actionable error feedback; a “View trash” toast action is sufficient for recovery using the existing route. Do not build an undo history.
- [x] Disable/hide mutation actions in read-only shares and require nonempty selections. Do not intercept Delete while typing or operating another interactive widget.
- [x] Verify targeted tests and `pnpm run check`; record evidence and update status.

**Acceptance:** Select a model and press Delete or choose its context action: it disappears and is recoverable in Trash. Selecting two models trashes exactly two. Typing Delete in the name/search field never trashes anything. Failure leaves rows available and the action usable. Group split-view deletes only the selected subset.

**Continue prompt:** “Verify the branch/worktree and execute step 3 of docs/ux-essentials-plan.md.”

## Step 3 — Add ordinary keyboard selection and activation

**Status:** Implemented and covered by focused tests.

**Files:** `view/grid-selection.svelte.ts`, `model-grid-inner.svelte`, `group-grid.svelte`, `selection-panel.svelte`; model/group editors for F2 name focus; `src/lib/utils.ts` only where its other resource-grid caller must remain compatible.

- [x] Test agent adds range-anchor, Ctrl/Cmd+A, Escape, Arrow keys, Home/End, Space, group Enter/double-click, and F2 cases to grid tests or a focused selection harness.
- [x] Keep an id-based range anchor across repeated Shift interactions; prune it when its item leaves the collection. Preserve existing additive Ctrl/Cmd click behavior.
- [x] Add Ctrl/Cmd+A for loaded items, Escape clear/close, Home/End, and keyboard traversal with roving focus. For wrapped visual grids, calculate vertical destinations from rendered positions; list mode advances one row. Shift extends from the anchor.
- [x] Add a labelled multiselect listbox owner and item checkbox labels. Keyboard focus must remain visible and return to the next available item after deletion.
- [x] Enter/double-click opens a real group regardless of member count; synthetic ungrouped entries continue previewing their model. F2 focuses the existing name field for one editable item. Leave other multi-selection activation unchanged.
- [x] Ensure Escape in nested controls closes that control first and does not unexpectedly clear unrelated selection.
- [x] Verify targeted tests and `pnpm run check`; record evidence and update status.

**Acceptance:** Repeated Shift clicks grow/shrink one range; keyboard-only users can select multiple models, invoke trash, clear selection, open a one-model group, and rename an item. Ctrl/Cmd+A in search still selects text. Tab traverses the collection as one composite control, not hundreds of items.

**Continue prompt:** “Verify the branch/worktree and execute step 4 of docs/ux-essentials-plan.md.”

## Step 4 — Make existing-group organization available without dragging

**Status:** Implemented and covered by focused tests.

**Files:** `edit/model.svelte`, `multi-model.svelte`, `view/right-click-models.svelte`; new `view/group-select.svelte` only if shared UI is required; reuse `src/lib/api/shared/group_api.ts` and existing Select/Popover components; model/group grid refresh callbacks from step 1.

- [x] Test agent covers single/multiple models, destination outside current selection, same-group no-op, read-only mode, and API failure with no optimistic corruption.
- [x] Add a searchable “Move to group…” control for one or many models, loading destinations through existing `getAllGroups`. Exclude synthetic negative-id groups and handle loading/error/empty results explicitly.
- [x] Reuse `addModelsToGroup(group, models)` for assignment and `removeModelsFromGroup(models)` for ungrouping. Show all real destinations, not only groups already represented in the selection.
- [x] Reconcile model membership and affected group lists after success so source cards/counts and the current group page stay accurate. Keep group assignment exclusive and labels unchanged.
- [x] Retain existing New group and Remove from group operations, with consistent text and disabled states. Use the same operation from the context menu and editor.
- [x] Verify targeted tests and `pnpm run check`; record evidence and update status.

**Acceptance:** A single ungrouped model can move into any existing group without selecting another member first. A failed move leaves original membership intact. Moving all members out of the current group produces a sensible empty/navigation state. Labels are preserved.

**Continue prompt:** “Verify the branch/worktree and execute step 5 of docs/ux-essentials-plan.md.”

## Step 5 — Repair dragging and add group destinations

**Status:** Implemented and covered by focused tests.

**Files:** `src/lib/drag-selected-models.svelte.ts`; `view/drag-selected-models.svelte`, `drag-selected-models-root.svelte`, `group-grid.svelte`, `group-page.svelte`; `src/lib/components/app-sidebar.svelte`; drag tests and sidebar tests.

- [x] Test agent writes source-to-root integration tests rather than only testing the five-pixel threshold: sidebar text and nested SVG/path targets, an in-grid group destination, selected batch, unselected source, unsupported destination, same group, Escape, blur, release outside app, read-only mode, and rejected mutation.
- [x] Give the root sole responsibility for resolving and completing a drop before clearing the snapshot. Resolve destinations with `Element.closest`, including SVG descendants. Avoid starting a drag on checkboxes, buttons, inputs, or blank collection space.
- [x] Keep the current custom internal drag model; do not switch the application to browser native file dragging and risk the existing Tauri OS import integration. Track movement/release at window level and clear state on cancellation/blur/unmount. Use pointer cancellation where pointer events are introduced; preserve ordinary touch scrolling and the non-drag control from step 4.
- [x] Add group drop targets to real group cards/rows and the active group's header. Ignore synthetic groups and no-op memberships. Use the existing group assignment API and reconcile the affected views after a drop.
- [x] Preserve sidebar label drops as additive labeling, avoiding duplicate labels. Highlight the actual valid destination and show “Add label” versus “Move to group” intent. Invalid targets show no success affordance.
- [x] Complete mutation and sidebar/collection refresh with explicit failure feedback. Cancelled drags never call an API, and mouseup cannot turn into an unintended card activation.
- [x] Verify targeted tests and `pnpm run check`; record evidence and update status.

**Acceptance:** Dragging one or several models onto a label's icon or text applies the label; dropping onto a group in the same grid moves membership exactly once. Source and target update immediately after success. Escape/window blur cancels cleanly. External OS file drops still import through Tauri.

**Continue prompt:** “Verify the branch/worktree and execute step 6 of docs/ux-essentials-plan.md.”

## Step 6 — Finish feedback, discoverability, and accessible controls

**Status:** Implemented and covered by focused tests.

**Files:** `view/model-grid.svelte`, `group-grid.svelte`, `search-input.svelte`, `right-click-models.svelte`, `selection-panel.svelte`; `edit/model.svelte`, `multi-model.svelte`, `group.svelte`; reuse `view/empty-state.svelte` and `spinner.svelte`.

- [x] Test agent covers empty library versus zero search results, clear-search/reset filters, initial loading, page-load failure/retry, accessible names, and absent platform capabilities.
- [x] Render distinct results states: loading, empty library with Import navigation, no matching results with clear-filter action, and failed load with Retry. Preserve existing rows during pagination failure.
- [x] Add a visible selection count and concise shortcut hints for the newly supported actions. Give search an accessible label and a clear button that immediately resets its debounce/filter.
- [x] Add accessible names/tooltips to changed icon actions and selection checkboxes. Hide or disable unsupported Open in folder/slicer actions based on the injected APIs, matching the editor behavior.
- [ ] Keep the existing visual system and mobile preview/back behavior. Confirm the action row wraps or remains usable at desktop narrow widths rather than introducing a redesign. (Visual inspection unavailable.)
- [x] Verify targeted tests and `pnpm run check`; record evidence and update status.

**Acceptance:** Users can distinguish an empty library, zero matches, loading, and failure, and can act to recover. Every changed icon control has a usable accessible name. Browser/share views do not offer actions that silently do nothing.

**Continue prompt:** “Verify the branch/worktree and execute step 7 of docs/ux-essentials-plan.md.”

## Step 7 — Verify the complete workflows and consolidate documentation

**Status:** Automated verification complete; visual and native verification unavailable.

- [x] Run all required frontend checks: `pnpm run check`, `pnpm run lint`, `pnpm run format:check`, `pnpm run test`.
- [x] Run the Svelte MCP autofixer on changed `.svelte`/`.svelte.ts` files if available, fix reported issues, and rerun it. If unavailable, record that explicitly.
- [ ] For each new behavioral guard, temporarily break its guarded line, demonstrate an assertion failure rather than a compile error, then restore and rerun the targeted test. Record this in the step evidence; do not retain broken code.
- [ ] Start the demo UI with `VITE_API_PLATFORM=demo pnpm run dev:web -- --host 127.0.0.1`. Exercise model grid, group grid, split view, group detail, label view, favorites, import results, and Trash. Verify keyboard focus/input safety and error recovery with mocked APIs. Demo evidence is not native Tauri evidence.
- [ ] Run a native desktop smoke test for OS file import plus internal drag, if a runnable desktop is available. Record any unavailable platform rather than claiming it passed.
- [ ] Verify at typical desktop and narrow widths, once in a batched inspection and once after fixes if needed. Avoid unrelated aesthetic changes.
- [x] Search changed names/behavior through comments, docs, and test descriptions. Add concise user-facing selection/organization guidance to the appropriate existing documentation, or turn this plan into a reference if none exists.
- [x] Obtain independent change review and fix valid findings. Keep human review required before merge; no push/merge is authorized.
- [x] Record final commands, results, skipped native/platform checks, and remaining product questions. Remove temporary files.

**Evidence:** `pnpm run check`, `pnpm run lint`, `pnpm run format:check`, and `pnpm run test` passed; the latest full frontend run covered 475 tests. `pnpm run build` passed with a chunk-size warning. `cargo fmt --all`, workspace Clippy, `cargo test -p db -p service` and the focused web share privacy test passed; one service helper test is intentionally ignored. The first sandboxed service run could not bind Wiremock ports; the permitted loopback rerun passed. The Svelte MCP was unavailable. Aikido could not authenticate without its API key or an accessible keychain. No browser was connected, so demo, narrow-width, screen-reader and native Tauri smoke tests were not run. Numerous new behavioral tests were observed failing before fixes or under restored mutations; a per-guard mutation audit remains incomplete.

The scoped Rust amendment below was implemented with a dedicated test agent and the required Rust checks.

### Amendment: include empty groups

Review found that the database and demo group listings derive groups from models, so empty groups cannot appear as destinations. Add owned empty groups to the existing listing with filter, sort, pagination, and resource semantics preserved; update demo parity and ensure empty group cards and details render safely. Dedicated agents cover the database and demo regressions. Verify Rust formatting, clippy and relevant tests in addition to the frontend checks. Outside-window drag release remains subject to native desktop verification.

Sync cannot distinguish a legacy empty group that was never visible from an intentionally deleted empty group without durable deletion records. Preserve unmatched empty groups instead of deleting them; this means an empty group deleted on one device may reappear from another. Empty groups must contain a model before project assignment, matching the existing resource API.

### Follow-up: durable group deletion sync

The user chose deletion records that propagate group deletion across devices, and chose to keep empty groups as destinations only. Desktop file-manager behavior has priority. Until the new protocol is implemented and verified, keep the conservative sync behavior above.

The user subsequently chose to keep models and ungroup them when their group is deleted, preserve concurrent offline group edits or additions, and make deleted groups recoverable for the existing Trash retention period. Restoring a group reattaches only former models that are still ungrouped. A concurrent edit that keeps a group removes its Trash entry. Older incompatible apps or servers must pause sync and request an upgrade. Minimal deletion records remain indefinitely. Competing offline moves of one model to different groups require a user choice.

When an older device's group cannot be proven to match an existing group, ask the user whether they are the same group; do not silently merge or assume they are separate. Competing offline group names also require the user to choose a name. These choices still require an implemented conflict interface and safe identity adoption before sync can be enabled.

If groups are confirmed to be the same but a model in each cannot be proven to share an identity, ask the user whether the models are the same. A group-identity decision does not imply a model-identity decision, and matching names or file contents alone must not silently settle it.

If the user confirms that models are the same but their files or details differ, ask which version and details to keep. Preserve the conflicting data until that choice is made.

**Progress:** Local group deletion, recovery, retention cleanup and web, desktop and demo recovery APIs are implemented with regression tests. The Trash interface displays and restores model and group rows independently. Database and account identities and local change revisions are stored, and owner-scoped consistent snapshots are available for future sync baselines. The causal sync protocol and conflict-resolution interface remain unimplemented. The architecture review identified a need for causal group and model-assignment history, owner-scoped idempotent operations, and a compatibility gate before sync writes; wall-clock timestamps alone do not establish concurrency.

Local model and project ids now use durable high-water allocation to prevent a stale selection from targeting a replacement created after the upgrade. The previous schema did not retain ids deleted before migration, so the new allocator cannot prove which ids were used before that boundary.

**Current safety gate:** The new client checks `GET /api/v2/sync/capabilities` before any sync write and remains paused even if a server advertises support. Do not enable it until causal reconciliation, retry safety, recovery, and protection against already-authenticated older clients are complete. Legacy password and token logins and writes from old authenticated sessions are rejected, but the full mutation and downgrade audit is still outstanding.

**Implementation checkpoints:**

1. Add owner-scoped durable deletion operations and expiring recovery data, with atomic deletion/ungroup and restoration tests; preserve the minimal history after recovery expires.
2. Record causal group and model-assignment baselines and implement conditional, idempotent server operations. Concurrent delete versus rename/add keeps the group; conflicting model assignments remain pending for explicit user resolution. Older unmatched groups are preserved during migration.
3. Add authenticated v2 sync, recovery and conflict APIs plus Tauri and demo implementations. Guard every mutation path, including import, MCP and already-authenticated legacy sync sessions. Do not expose private history through share APIs.
4. Exchange and resolve group lifecycle changes before any existing model-sync writes. Store unresolved assignment conflicts durably and provide a user choice; never overwrite a later move on restore. Reject unsupported servers and old clients before writes.
5. Verify multi-device and out-of-order delivery, retries, crash recovery, upgrade and downgrade compatibility, retention, ownership and shared-view isolation. Then remove the temporary empty-group sync workaround and document the final protocol.

Assignment baselines must be keyed by local user and a stable server-instance/account identity, with model and group global ids. A missing baseline is distinct from an ungrouped model. Inspect both sides before any model sync writes; preserve divergent moves as pending user choices. Apply individual memberships with transactional compare-and-set on both sides, retry idempotently, and advance the baseline only after verified convergence. Whole-group replacement and timestamp comparisons cannot safely resolve these conflicts. A group deletion also needs causal history to distinguish an earlier edit from a concurrent one.

For three or more offline devices, record causal events and per-entity frontiers for group metadata/lifecycle and model membership. A local user action gets a durable replica id and counter; replication preserves the original event rather than creating a new user action. A deletion keeps its causal context after Trash expires. Concurrent membership choices remain pending until a resolution observes both branches; concurrent deletion versus rename or addition keeps the group. Tag a deletion's derived ungrouping so it is distinguishable from an independent move. Out-of-order events must retain or request their dependencies. Upgrades cannot reconstruct historical causality: resolve ambiguous first-sync state conservatively. Replica identities must also handle database restoration or cloning before new events are issued.

The membership comparison and stale-choice checks are implemented and tested independently, but are not connected to sync. Causal events, persistent baselines and choices, the conflict interface, the server endpoints and the new sync coordinator are still missing. Current local revision heads provide compare-and-set protection only; they must not be treated as causal clocks.

A database-held instance identity survives backup restoration and cloning. It therefore cannot alone detect a rollback or distinguish two independently operated clones; sync must fail closed or use an external incarnation mechanism for those cases before the capability is advertised.

An owner-scoped batch now checks model incarnations and group generations under a write transaction, journals exact retries, and combines group lifecycle changes with guarded membership changes. A purged group's old members cannot be recovered unless a replica still knows them. The internal Keep operation can preserve an explicit project association when the local project identity is verified; resource identity and concurrent resource edits still need a cross-device causal protocol before enabling sync.

### Causal history implementation guide

This section describes the remaining work. The existing group heads, deletion records, assignment revisions and conditional batch do not by themselves establish causality or make sync safe to enable.

**Entities and schema.** Store typed group events and model-membership events in owner-scoped durable tables. A group entity and a membership entity each need a shared birth identity, distinct from local compare-and-set incarnations. Each event carries its entity, an immutable dot `(replica_id, counter)`, explicit parent edges, its typed action and any required cross-entity dependency. Store the per-entity frontier transactionally, and validate every edge and dependency before applying an event. A global replica counter can provide unique dots, but its maximum is not proof that earlier events from another entity were observed.

The local event recorders now cover ordinary renames, moves, group additions, deletion and restoration. They allocate a global replica counter and store explicit parent links, but the event DAG is incomplete; no reader may infer that a counter includes earlier events from another entity. All histories remain marked unknown, and replication must stay disabled until the representation and every mutation path are reconciled with this design.

An internal rename inbox stores unresolved dependencies without changing group state. It is not a sync endpoint or an authentication mechanism. Replica credential binding, wire-stable group incarnations, owner-scoped storage quotas, paginated reads, the other event types and a causal reducer remain required before exposure.

The account-pinned full snapshot is currently an internal, unbounded read; its HTTP prototype is compiled only for tests and is not registered. Before exposing it, support consistent, owner-bound resumable pagination with explicit row, byte, storage and lifetime bounds, or reject oversize requests without returning a partial snapshot. Never paginate independently over changing live tables and call the result one snapshot.

**Astra review:** Keep the hard pause until these stages are complete: (1) define shared births, writer identity, authenticated relay and a conservative legacy handshake; (2) complete creation, project, lifecycle, membership and resolution events with an order-independent reducer; (3) provide one bounded typed inbox and authenticated relay that preserves original event authors and applies receipts, state and frontiers atomically; (4) persist per-account/server cursors, pending choices and retry state; (5) connect the conflict UI and revalidate all displayed branches when a choice is submitted; (6) pass three-device, upgrade, retention, crash, account-switch, clone and share-isolation tests before advertising capability. The pairwise membership helper is a prototype, not the authority for multi-device decisions. Audit MCP's independent bearer-authenticated mutation path as part of compatibility coverage.

New local groups and models have portable birth identities, while pre-existing rows remain explicitly unknown. The internal Keep operation can rehydrate a purged group only when its retained deletion record and registry match the supplied birth. Verified cross-device adoption and replica authentication are not implemented.

Legacy identity adoption must use evidence from an authenticated peer and an owner-scoped receipt bound to the complete observed local state. Recheck the local incarnation, revisions and related group/project state in the same transaction before assigning a birth; same-second timestamps or model assignment revision alone cannot prove an observation is unchanged. Adoption must never mark unknown causal ancestry complete.

New local group and model creation writes an isolated record in the same transaction. These records are explicitly incomplete and non-authoritative. Existing rekey paths can change an entity's global id after its creation record was written; before exporting any record, block mismatches until identity binding and rekey semantics have been resolved.
The current envelope bounds group names to 65,536 UTF-8 bytes; creating a group with a longer name now fails atomically. Review this limit as part of the final protocol and product compatibility check.

Group actions must represent creation, metadata changes including rename and project association, deletion, restoration referencing the exact deletion dot, and membership additions needed to compare a concurrent add with deletion. Membership actions identify the target group or ungrouped state and distinguish an independent user move from a detach derived from a specific group deletion or an attachment derived from restoration. Model trash state and group-id reuse require explicit treatment in the entity and event contracts; never associate an old deletion with an unrelated new incarnation.

**Local mutation transactions.** Route actual group creation, rename, deletion, restoration, model assignment and their batch, import and other mutation paths through shared transactional primitives. Capture each action's observed frontier inside the write transaction, allocate its dot, update the domain rows, insert its events and advance its frontier atomically. A group deletion must write its group event and a tagged detach event for every affected live or trashed model in the same transaction. A local Trash restore emits membership events only for former members actually reattached. Record membership additions in the affected group's causal history as well, or define an equivalent explicit cross-entity dependency: the group history must distinguish an add observed before deletion from a concurrent add. No-op retries must not create a new user action.

**Replication and resolution.** Ingest typed remote events with their original dots and contexts; do not mint local events for replication. An exact duplicate is idempotent, while a reused dot with different content is rejected. Persist missing dependencies without advancing the affected frontier or applying a dependent domain change; resume causal processing when the dependencies arrive. Validate owner, entity, model incarnation, context shape, and referenced deletion for derived detaches; bound untrusted event and dependency input and reject cycles or invalid self-dependencies. Store competing frontier branches until reconciliation: concurrent delete versus rename or addition keeps the group; concurrent model moves and different concurrent group renames require explicit user choices. A resolution event must observe all branches it resolves, and stale choices must be revalidated. A group retains its existing project association when all its models are in Trash. Preserve deletion context and required event dependencies after Trash recovery data expires. A purged group can recover members only from a replica that still knows them.

**Upgrades and replica identity.** Mark entities predating causal history as having unknown history; an empty history or a fabricated genesis must not imply that separate devices observed one another. Establish a conservative first-sync baseline before treating missing history as known. A writable database initialization now rotates the random writer id and resets its counter transactionally, preserving existing events and account identities. This prevents a normally reopened consistent backup or copy from reusing its former writer counter; concurrent pools allocate from the current database writer. It does not detect replacement under an active writer, process or VM memory clones, or stale restored sync checkpoints. Define and enforce the supported restore workflow, writer registration and checkpoint validation before advertising sync. Handle ambiguous legacy duplicate identities without inventing an ordering.

**Implementation and test gates.**

1. Add read-only owner-scoped typed history/frontier access and explicit unknown-history state. Before instrumentation, write migration tests for live, trashed and purged legacy entities, duplicate identities and restart persistence.
2. Instrument the existing local mutation entry points. Test typed actions and contexts, owner isolation, same-second changes, no-op retries, exact deletion references and independent versus deletion-derived detach. Inject failures on event insertion and domain writes to prove events, counters, frontiers and domain changes roll back together. Audit every group and membership write path, including batch, import and cascades, before claiming coverage.
3. Add typed replicated ingestion and durable pending dependencies. Test original-dot preservation, exact retry, conflicting duplicate dots, out-of-order delivery across restart, missing dependencies, invalid contexts and owner isolation. Test replica rekey and refusal to issue events when clone safety cannot be established.
4. Exercise three offline replicas from a verified shared baseline: one deletes a group, one renames it, and one adds a model. Deliver events in different orders with retries; verify concurrent branches survive, delete-derived detach differs from an independent move, and an observing resolution converges. Separately test conflicting moves, recovery after retention expires, and crash/retry at each transactional boundary.
5. Only after the reducer, conflict UI, authenticated transport, baseline binding, clone handling and compatibility audit all pass end-to-end tests may the server advertise the new capability and the coordinator resume sync. Keep share routes free of private history.

- [ ] Specify conflict and recovery behavior, including devices that stay offline longer than the Trash retention period.
- [ ] Design owner-scoped durable records, atomic deletion/apply and retry-safe synchronization, with an explicit old-client/server compatibility strategy.
- [ ] Add tests for upgrade, ownership, retries, conflicting offline changes, and deletion propagation before implementing each affected surface.
- [ ] Implement and verify database, desktop, web and sync integration, and document the resulting behavior.

### Next stage — shared identity, event contract and initialization

**Status:** Design documented; the checklist below is not implemented. Existing portable births, local incarnation guards, partial event recorders and rename inbox are foundations only. This stage does not enable synchronization.

**Branch/worktree:** `feat/ux-essentials` in `/home/tanguille/Documents/PriveProjecten/mesh-organiser`. Follow the branch checks and Process Instructions above, including the user's authorization to continue through steps. A dedicated test agent owns regression tests for each affected surface, explains the behavior, lists concrete cases, establishes the failing/baseline evidence and reports the executed checks. After each checkpoint, update its status and evidence before proceeding.

#### 1. Define the portable contract

- [ ] Add shared Rust protocol types in `db/src/sync_protocol.rs` and export the module from `db/src/lib.rs`. Use a closed tagged action enum and reject unknown fields. An `EntityRef` contains entity kind, global id and portable birth; an envelope contains protocol version, entity, immutable author/writer dot, same-entity parents, typed cross-entity dependencies and action.
- [ ] Use `group_birth_id` and `model_birth_id` for portable identity. Include a birth in every model/group reference, including membership destinations, additions, detach and restore. Keep numeric ids, `model_incarnation`, group generation and revisions local to compare-and-set operations. Resource identity requires its own subsequent portable contract.
- [ ] Preserve the explicit-parent DAG. Sort and validate parent/dependency sets, reject duplicates and invalid self-dependencies, and require all dependencies before application. Counter maxima never imply observation. Encode counters as decimal strings on the wire to preserve values beyond JavaScript's exact integer range; check the supported SQLite integer range.
- [ ] Define canonical typed serialization for exact duplicate comparison. Ownership comes from authenticated account binding, not an envelope's numeric user id. A reused owner/writer/counter with different content is an error.

**Acceptance:** Contract tests reject malformed identities, counter overflow, ambiguous references and conflicting duplicate dots. Group/model snapshots export portable births while retaining separate local guards.

#### 2. Persist identity binding without inventing history

- [ ] Extend `entity_births` with initialization kind (`unknown`, `created`, `legacy_observation`) and an optional initialization dot. Existing registry rows default to `unknown`; a nonnull birth does not prove complete history. Keep existing null births null until verified adoption. The `unknown` and `created` cases are implemented and tested, including a guarded match to the exact local creation record. Rekeying resets initialization to unknown while retaining the old evidence; `legacy_observation` remains to be implemented with adoption.
- [ ] Add `sync_peer_bindings`, keyed by local user plus server-instance id and remote-account id. Reuse the server identities already exposed by `sync_instance` and `sync_accounts`; a URL or numeric remote user id is not an account binding.
- [ ] Add durable `sync_identity_adoptions` receipts containing binding, entity kind/global id/birth, local incarnation or generation guard, observed revision, canonical observation digest and receipt id. The receipt must refer to the exact legacy state checked by both peers.
- [ ] Implement `describe_sync_identity`, `describe_legacy_entity`, `prepare_identity_adoption` and `commit_identity_adoption` as internal owner-scoped operations first. Perform receipt validation, null-to-birth adoption and registry update in one write transaction. Update immutable-birth triggers to permit only this guarded first adoption; nonnull-to-different-birth remains forbidden. Ordinary edit APIs cannot adopt identities.
- [ ] Preserve exact retries, reject stale local observations and restart the handshake when a row, revision, incarnation or deletion operation changed. Do not infer identity solely from names, membership or blob hashes. A global-id match identifies a candidate that must also pass duplicate, retained birth and deletion-record checks.

| Observed state                                  | Required result                                                                                            |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Same verified birth/global id                   | Same entity; compare history separately.                                                                   |
| Different nonnull births under one global id    | Block automatic merge as an identity collision.                                                            |
| Missing or duplicate global identity            | Block adoption.                                                                                            |
| Legacy null birth with verified unique identity | Adopt once using the guarded receipt; ancestry remains unknown.                                            |
| Unmatched legacy entity                         | Preserve it; absence proves neither deletion nor equivalence.                                              |
| Late offline legacy state                       | Retain an unknown-history observation; never fabricate descent from a baseline the device did not observe. |

**Acceptance:** Migration, ownership, duplicate identity, stale receipt, rollback, retry and restart tests pass. Adoption establishes portable identity only; it cannot mark history complete.

#### Snapshot transport gate

The internal full snapshot is a coherent read, but it is not yet a bounded or resumable transport. Its web route remains unregistered. Do not advertise the capability or connect sync until large-library reads are complete and verified.

A bounded one-shot response can serve only as a temporary fail-closed guard: validate the pinned account first, count owner-scoped rows and preflight projected text bytes in the same read transaction, then enforce the exact encoded payload limit with a bounded serializer before returning any body. Exceeding a row or byte cap must return an explicit error, never a truncated snapshot. Readers interrupted during this operation restart from the beginning, and libraries above the cap remain unsupported.

For resumable large-library transport, materialize an immutable snapshot under the authenticated owner, instance and account identity, and publish it only after the complete point-in-time copy commits. Persist stable section and item sequence keys so short, bounded keyset pages can be retried or resumed after a process restart without mixing versions. Every page must recheck the authenticated owner and pinned account; a snapshot id is a locator, not authorization. Include schema version, section totals and an end marker to detect gaps or truncation. Expiry, quota exhaustion, cancellation and account replacement must return explicit errors; a replacement snapshot must never silently reuse an old cursor.

Bound each snapshot's rows, encoded bytes, copy time and disk use, plus per-owner concurrent snapshots and retention. Use database-side staging rather than collecting the entire library in application memory, and make incomplete staging invisible and recoverable on failure. An arbitrarily large copy cannot also have a strictly bounded transaction: if supporting data beyond the materialization cap is required, first implement versioned as-of reads across short transactions. Paginating live tables without a stable version is not a coherent snapshot.

Before enabling this transport, test exact row and encoded-byte boundaries, escape-heavy and oversized fields, concurrent writes between pages, retry and restart, owner/account replacement, cursor tampering, incomplete staging, cleanup races, quotas and cancellation. Verify every section and row appears exactly once and all pages refer to one read view.

#### 3. Establish one authoritative event store

An internal, bounded receipt-only store now persists canonical incoming envelopes and dependencies, checks exact retries and cross-store dot collisions, and recomputes dependency and identity status without applying events. Conflicting claims are retained; unknown legacy identity evidence remains unknown. It validates the caller's captured local instance and account identities inside each transaction. It is not authenticated transport or an authoritative reducer; an authenticated wrapper must supply that captured identity. Conflicting claims still need authenticated provenance and a resolution or quarantine workflow before application. Its current quotas have no cleanup or pagination, and old partial histories cannot yet satisfy portable dependencies. Purges and rekeys predating the new retained markers cannot be reconstructed when no evidence survives. Sync remains paused.

Local creation records and remote receipts now share one immutable event payload and dot namespace, with separate origin metadata. Migration preserves original bytes and fails atomically on duplicate dots; creation remains explicitly incomplete. Existing group and membership histories still use legacy representations and have not been promoted to portable frontiers.

The existing local group and membership chains now use the shared journal with a local-only incomplete encoding. Their original dots, actual parents and local tips are preserved, and portable receipts reject this encoding as verified ancestry. Older partial chains are not attached to creation events they did not observe. Rename, move, delete and restore journal writes remain atomic with their domain changes. The internal sync batch, project association, model lifecycle and rekey paths are not fully represented; they remain explicit coverage gaps until handled by the reducer and portable contract.

An internal pristine-genesis path can enroll and install an exact creation event across independently initialized databases while preserving its birth and event identity. It is restricted to verifiably untouched entities; existing models without a durable initial eligibility guard remain ineligible, and metadata, label or blob-descriptor changes invalidate model eligibility. A peer pin is not authentication, and matching stored blob descriptors does not verify file bytes. This narrow path does not adopt existing edited libraries or enable synchronization.

For an unambiguous active group, an internal observation path can enroll an edited existing row and install its exact `LegacyObserved` roots on another database. It preserves active and trashed member models, exact observed Trash timestamps, matching local blob descriptors, direct label assignments and their connected label hierarchies, and the checked project association with its raw metadata; observed initial memberships and Trash states are not fabricated events. Shared label and project reuse requires matching stored observation receipts. It preserves or guardedly assigns births, retains unknown earlier ancestry and rejects stale captures and retries. Later model lifecycle changes pause application; occupied matching targets, later label/project edit exchange, physical blob verification and authenticated exchange remain unsupported. This path does not enable sync.

An internal rename-only reducer now records real renames for a verified pristine group, retains competing branches, and requires a current branch token for an explicit name choice. It preserves the visible name while history is pending or conflicted. Unlike the generic receipt store, this scoped apply path requests missing parents without admitting or acknowledging the child; the sender must retain and retry it. Capacity exhaustion pauses the scope without discarding its evidence. This is not authenticated transport or a general sync path: membership, deletion, project and other unsupported changes invalidate the scope, and enrollment of an edited existing library remains unresolved.

An internal move-only reducer now records real model moves from verified pristine genesis and retains competing destinations, including ungrouped. Resolution requires a current token and an offered destination; imported resolutions are checked against their parent choices too. Unsupported lifecycle and identity changes pause or invalidate the scope while retaining choices. Moves currently invalidate the separate rename-only group scope: composing group rename, membership addition and deletion still needs a shared reducer before sync can resume. Matching blob descriptors do not authenticate a peer or verify file bytes.

When a concurrent rename or addition defeats a deletion, its derived detach is inactive rather than a new assignment back into the group. This distinction prevents a later independent move from being undone. The combined reducer must validate actions against their recorded causal past and clear the losing deletion's Trash projection atomically with the resulting group and model state.

An internal combined reducer now exercises those rules for explicitly enrolled pristine entities: real move, rename, delete and restore transactions retain original events, reconcile concurrent branches and remove losing Trash entries while retaining deletion records. Certified retention purges can rehydrate the same group birth with a fresh local id when a later concurrent event keeps it; independent moves are preserved and absent model or project data is not invented. Focused tests cover both delivery orders, repeated purge/recovery, later independent moves, choices, missing dependencies and rollback. This remains a bounded internal checkpoint: unverified historical absences, existing-library adoption, authenticated exchange and capacity checkpoint/resume are not implemented, so sync remains paused.

A test-only authenticated hub now exercises group genesis, rename, complete model-move/add, destination-resolution, delete and restore exchange through real sessions, server-issued writer credentials, original-event relay and bounded cursor reads. New delete/restore operations carry an immutable authored-context manifest; receivers validate its causal closure and request missing dependencies without acknowledging the operation. Historical operations without such context remain blocked. Account-scoped offline writer delegations preserve new original event authors across restarts and consistent database copies. The controller is not mounted in production. A local peer pin alone does not authenticate a remote endpoint; the desktop coordinator, verified transport, populated-target reconciliation, file verification and applied-cursor persistence remain required before enabling sync. Events from writers predating authority registration remain preserved but cannot yet be published.

The test-only hub can also accept a bounded observed group and verified logical file bytes, preserving labels, project and Trash state, then serve owner-scoped verified downloads. A test-only HTTPS client verifies its configured origin and hub identity, applies the supported ordered feed kinds to a third database, and commits each receipt and cursor with its data. Exact previously verified deliveries can be acknowledged on a new stream without overwriting later local edits, even after hub file retention expires; unrelated or unknown targets remain blocked. File staging uses owned paths, durable intents, leases and directory sync on supported Unix platforms; recovery preserves unverified replacements. Whole requests retry rather than resume, and an abrupt stop before recording a staging intent may leave an empty reservation. Non-Unix transfer, general populated-target reconciliation and production integration remain gated. Subprocess tests cover crash boundaries, but do not prove physical power-loss behavior.

The test-only pull also accepts an occupied target with a verified identical original baseline and independently authored portable changes, after matching archived roots and metadata and verifying the existing and incoming file bytes. It preserves the local state and exposes conflicting names and destinations through the internal reducer; stale choices are rejected. Previously attested content is reused only when its full descriptor and blob binding match. Contradictory issuer or digest evidence is rejected, and a receipt from another server cannot replace fresh file verification. HTTPS tests cover these cases, atomic receipt failure, and unchanged project, labels and Trash state. Unsupported metadata drift and unknown ancestry still pause; this does not enable application sync or provide a conflict-resolution UI.

The internal HTTPS pairing path now authenticates the server and account, registers or checks a writer credential, and atomically records the local link and authority only if the captured account and link are still current. Failed or cancelled pairing retains the previous local link; an unused remote grant can remain after cancellation. Seven real HTTPS pairing tests and all 23 existing HTTPS pull tests passed, including retry, concurrent pairing, revoked credentials and account replacement. Three pairing safeguards were confirmed by mutation; an early account revalidation guard was not independently proven by that check. This path is not yet connected to the desktop link action or a production server route; sync remains paused.

An internal HTTPS publisher now sends selected original genesis records, complete operations and prepared observations with their original device evidence. It returns missing dependencies without acknowledging them and verifies successful publication against the authenticated log before returning. Three HTTPS tests cover paired-device publication and pull, account isolation, retries, corrupt bytes, stale links and forged acknowledgments. Six targeted mutations to the acknowledgment, link and content checks produced assertion failures. If the server commits before the local link changes, the client rejects the acknowledgment; an exact retry can recover. Automatic scheduling remains necessary before production sync can be enabled.

A selected-item durable outbox now retains the original record and certified file stage across restart and later edits. Enrollment and queue insertion share a transaction; an authenticated exact acknowledgment compacts the queued payload and releases the stage reference. Missing dependencies and stale links remain pending, and the owner can explicitly discard stranded work without marking it synced. Stages use a bounded pool of certified directories and permanent leases; cleanup uses directory-relative operations without deleting a replaced path. Nine HTTPS outbox tests and 28 subprocess crash/recovery and reuse tests passed, including interrupted commits, competing processes, replacement paths, full pending capacity, and single-connection database pools. Uncertified reservations and legacy intents fail closed and consume the finite slot limit; older forgotten directories are not adopted, and non-Unix transfer remains unsupported. A copied database cannot currently discard a pending stage certified for the original database: its pending reference remains blocked and charged against capacity. Recovery leaves foreign certificates and files untouched while allowing independent certified staging. Replacing a SQLite pathname under a live connection, inode reuse/ABA and restored checkpoints remain unproven ownership boundaries; pathname metadata is not proof of the inode held by that connection. Automatic enqueue, scheduling, production routes, the desktop coordinator and conflict UI remain unfinished, so application sync stays paused.

Stage allocation checks SQLite's live connection for a moved database and fails closed when that check is unavailable. A regression covers replacing the database pathname while the old connection remains open. This narrows a demonstrated failure; it does not establish protection against pathname ABA, mixed connections in a pool, or restored checkpoints. Those remain activation blockers.

The internal identity-confirmation path now supports one narrow case: an active empty legacy group with no project and an unknown birth can be matched to an authenticated observed group with the same unique global id. Confirmation retains independent incomplete observations; differing names remain an explicit conflict and the observation can be relayed. A keep-both decision can instead assign a fresh observed identity to the local group and install the remote group separately, preserving the prior head as evidence. Both paths retain exact retries and reject stale choices. Seventeen HTTPS tests passed for choices, relay, ownership, rollback and unsupported candidates; separate tests cover a third device making a name choice stale and conservative rejection of valid, unrelated, malformed and non-UTF8 batch journals. A snapshot-recheck mutation was not independently caught, although database revision and birth-adoption guards also reject stale snapshots. Separation rejects retained group or membership history and any owner assignment-batch journal. The user-facing identity question and matching populated or otherwise ambiguous groups are not yet implemented; sync stays paused.

The internal choice also accepts a populated remote group when the local candidate is empty and all incoming model identities are novel; remote labels or projects remain unsupported. Both choices download and verify the original files, then recheck the decision and child identities inside the transaction that commits models, observations, receipt and cursor. Focused HTTPS tests cover retries, collisions, legacy child-history references, changes during download, corrupted files and rollback. Existing populated-install, staging and pull regressions also passed. The additional legacy-history guard was confirmed by mutation; a later collision-check mutation survived because the installer repeats the check in the same transaction, as independently reviewed. The desktop prompt remains unsupported.

The internal keep-both choice also supports populated local groups with distinct child identities, provided the guarded legacy state has no unsupported labels, projects or retained group-history references. It retains local rows and verified file bytes, installs the remote group separately, and commits the local durable outbox entry with the incoming receipt. Twenty-eight identity HTTPS tests, nine outbox tests and 29 staging/recovery tests passed; mutation checks caught removal of staged-file retention and the local snapshot recheck.

Confirming two populated groups as the same is also supported internally for disjoint, novel model identities and no unsupported labels or projects. Each original membership snapshot remains an independent observed root with its own batch, and a verified supplemental batch can be relayed. Focused tests cover both orders of concurrent group deletion, rehydration after retention expiry, independent moves, malformed baseline references and three-device relay; mutations caught removal of batch validation and standalone-batch rejection. Overlapping or unproven model identities and the desktop prompt remain unsupported; application sync stays paused.

New empty-group decisions and name choices now commit their exact outgoing publication in the same transaction as the decision. Exact retries preserve the publication and return the current name-conflict state; explicitly discarded work reports a discarded error rather than a nonexistent queued item. Existing decisions created before this change can still lack a queued publication. Forty-eight identity HTTPS tests passed, with targeted mutation checks for empty-decision and name-choice enqueue and nonempty file-stage enforcement. A receiving device can also prepare a name choice using its authenticated applied identity-observation receipt; resolution rechecks the receipt and conflict token, then commits the name event and publication together. Eight focused HTTPS tests passed, and mutations confirmed the receipt and digest checks. Production transport, desktop coordination and the prompt are not connected.

An internal bounded coordinator now examines one authenticated feed item and either applies that exact item, offers an eligible identity choice, or reports the current feed boundary. It does not report account-wide completion. Pending publications are listed separately in account/link-bound pages; newly inserted entries before a page cursor require a fresh scan. Focused HTTPS coordinator and pagination tests passed, and mutations caught an extra fetch, omitted historical-echo check and ignored page cursor. The coordinator is not wired into the desktop or a production route; ambiguous model identities outside the narrow explicit decision below remain blocked.

An internal explicit model-pair decision now handles one narrow legacy case: one active, history-free candidate per group, the same model global id and an unknown local birth. The group question is answered first and bound to its exact offered delivery; a pending answer can be canceled and restarted with a fresh token. Same-model confirmation requires matching metadata and verified bytes, retains independent observations and relays them. Separate-model confirmation preserves both rows and files with incarnation-specific identity evidence. Separate groups with the same model require an unresolved membership choice and do not commit. Focused tests cover consent, retry, stale and canceled tokens, later moves, third-device relay and atomic rollback; an authenticated forged cross-group model baseline is rejected. Mutations caught removal of its provenance check and weakening the incarnation evidence. Multiple ambiguous models, different versions and the user-facing prompts remain unfinished; sync stays paused.

Newly authored supported lifecycle operations with an existing authenticated writer delegation now receive durable selection markers in the local edit transaction. A bounded selector can queue the original complete operation later; a full queue leaves the local edit and marker intact, and an explicit discard is not silently requeued. Older untracked and other-link records remain distinguishable rather than assumed published. Ten focused selector tests and an original-event relay test passed; targeted mutations caught missing operation recording and discard disposition. This covers the combined lifecycle journal only and does not establish whole-account sync completeness.

- [ ] Introduce a common `causal_events` envelope store with owner/writer/counter uniqueness, entity kind/global id/birth, canonical envelope, receipt/application state and a local pagination sequence. The pagination sequence is not causal order.
- [ ] Store parent and typed cross-entity references in `causal_event_dependencies`; persist unresolved references without requiring the referenced event to exist yet. Key `causal_entity_frontier` by owner and full portable entity identity. Validate dependencies again when they arrive, including owner, entity, birth, type and cycle checks.
- [ ] Port local recorders/readers to the common representation without running two independent authoritative histories. Preserve the current partial history as explicitly legacy evidence during migration. Replace the rename-only inbox with typed ingestion as its tests are ported; do not add separate authoritative inboxes for every action.
- [ ] Bound envelopes, dependency lists and owner storage; define paginated reads. Persist accepted events and pending dependencies across restart. Application, materialized domain state, frontier and application checkpoint must eventually share a transaction; receipt alone must never imply application.

**Acceptance:** Exact retry and conflicting-duplicate tests cover different action types, not just renames. Missing dependencies survive restart and cannot advance the applied frontier. Existing partial histories are not silently converted into complete v2 histories.

#### 4. Record genuine creation and legacy observation separately

- [ ] Define `GroupCreated { name }`, `ModelInitialized { group: None }` and `LegacyObserved { observed_state, observation_digest }` actions. Initialization payloads contain portable references, never local blob or row ids.
- [ ] Extend `add_empty_group` to create its row, birth, creation event and initial frontier in one write transaction. Extend `add_model`'s existing transaction to record the initial ungrouped membership and frontier. Inject event/domain write failures and prove full rollback.
- [ ] Add an internal replicated-creation operation that preserves the sender's global id, birth and original event while allocating fresh local row ids/incarnation. Do not create a different entity and rekey it afterward.
- [ ] Emit `LegacyObserved`, never a fabricated creation event, for adopted legacy state. Retain existing verifiable local tips as dependencies where applicable, and keep unknown ancestry explicit. An empty parent list or successful adoption cannot establish complete ancestry.
- [ ] Require subsequent moves to observe their model initialization and depend on the target group's creation or verified legacy observation. Cover imported models, empty groups, deletion/recovery and purged identity registries in the creation/adoption tests.

**Acceptance:** Two databases retain the same portable birth and original initialization event while using different local ids/incarnations. Pre-upgrade observations remain distinguishable from genuine creation. `created` records an authentic beginning, not proof that every mutation path is already instrumented.

#### 5. Define authenticated authorship and relay

- [ ] Separate `ingest_authored_events(authenticated_writer, envelopes)` from `ingest_server_relay(bound_server_account, envelopes)`. Server uploads authenticate the account and registered writer; new authored events must belong to that writer. Client downloads trust only the bound authenticated server/account and preserve each original author.
- [ ] Replace the rename inbox's sender-equals-author assumption in the typed path. Device C must accept device B's event relayed by the server without minting a C/server event or letting clients forge arbitrary historical authors.
- [ ] Specify writer-incarnation registration and restore/clone handling before uploads exist. A copied database cannot resume issuing under a copied writer incarnation; rotate through a verified rekey workflow or fail closed. Database-held identity alone does not prove clone safety.
- [x] Rotate the writer on normal writable database initialization. Four focused integration tests cover reopening, consistent copies, concurrent pools, preserved history and initialization failure. Registration of historical writers, active replacement and restored checkpoint validation remain open.
- [x] Preserve local writer identities across rotation and prevent the internal rename inbox from accepting them as remote authors or reusing a retained creation dot. This guard is not authenticated writer registration or a relay protocol.
- [ ] Define the future authenticated v2 identity/adoption and paginated event exchange API contracts, with owner-scoped receipts and original-envelope retries. Keep these separate from share routes and ordinary model/group edit APIs. Audit the independently bearer-authenticated MCP path before any capability is advertised.

**Acceptance:** Tests exercise original-author upload, authenticated relay to a third replica, forged-author rejection, foreign-account rejection, duplicate delivery and writer-incarnation refusal after an unsafe restore/clone. No event signatures are required when the central authenticated server is the trusted relay.

#### 6. Verify this stage and retain the activation gate

- [ ] Run focused contract, migration, identity, initialization and ingress tests with a separate test agent. Demonstrate assertion failures under restored mutations for the new guards. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets` and the affected Rust test suites; run frontend checks if wire consumers change.
- [ ] Exercise a late third legacy replica: identity adoption must not silently make its state descend from an earlier two-device baseline. Test receipt retry after restart and failure between local initialization and commit.
- [ ] Update this plan with commands/results and remaining unknowns. Confirm that the client still hard-pauses and the server does not advertise the capability. Full mutation coverage, resource/project history, the causal reducer, persisted coordination and the conflict UI remain later gates.

**Continue prompt:** "Verify the branch/worktree and execute the next unchecked checkpoint in the shared identity, event contract and initialization stage of docs/ux-essentials-plan.md. Keep sync paused and use a separate test agent."

## Verification commands by surface

Latest verification on 2026-11-17: frontend check, lint, format check and all 555 tests passed; the db, service and web test suites passed, including 301 web tests. `cargo fmt --all --check` and Clippy for db, service and web passed (with an existing web warning). Workspace-wide Clippy could not build the desktop target because the OCCT cache was absent and its source download could not resolve in the sandbox. Native and visual desktop smoke tests remain unverified. Production sync remains unavailable: the capability endpoint and replacement coordinator are not mounted, and the desktop client intentionally pauses sync. Ungrouped model cards in Groups now also support Delete and context-menu trash; real-group and mixed selections remain protected.

```bash
pnpm exec vitest run src/lib/components/view/grids.test.ts
pnpm exec vitest run src/lib/components/view/drag-selected-models.test.ts
pnpm exec vitest run src/lib/components/edit/model.test.ts
pnpm exec vitest run src/lib/components/app-sidebar.test.ts
pnpm exec vitest run src/routes/import/page.test.ts src/routes/trash/page.test.ts
pnpm run check
pnpm run lint
pnpm run format:check
pnpm run test
```

Add newly created focused test paths to the relevant step's recorded commands. Tests use the existing Vitest/jsdom and Testing Library patterns, stub APIs, and never delete real user models.

## Deferred or unverified

- Filesystem folder moves, nested group hierarchies, clipboard cut/copy/paste, permanent-delete shortcuts, custom hotkey configuration, drag reordering, and a global undo system are outside this elementary repair plan.
- Desktop WebKit, Windows/macOS key conventions, touch dragging, screen-reader output, and visual contrast were not exercised in this code audit. Browser and native checks above must report their actual coverage.
- No health score is assigned from source inspection alone; it would imply evidence the audit did not collect.
