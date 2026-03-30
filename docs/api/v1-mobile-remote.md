# HTTP API — v1 mobile remote client

Contract note for **Tauri mobile** talking to a remotely hosted **`web`** (Axum) instance. Canonical product decisions live in [2026-03-29-remote-client-and-http-api-design.md](../superpowers/specs/2026-03-29-remote-client-and-http-api-design.md).

**CORS:** Browsers enforce CORS on cross-origin `fetch` / XHR. The **`web`** server sends `Access-Control-Allow-Origin` for an explicit allowlist (default localhost dev URLs, plus any origins from **`MESH_ORGANISER_CORS_ORIGINS`** — see [commands.md](../commands.md#web-server-web-crate)). **Native** HTTP clients (e.g. **Tauri `tauri-plugin-http`**) are not subject to browser CORS; only browser-based or WebView `fetch` to a different origin needs the server allowlist to include the client origin.

### HTTPS in production

**Prefer HTTPS** for the user-configured remote base URL in production: TLS protects credentials and payloads on the wire, and session cookies that must be cross-site often require **`Secure`** (and thus HTTPS). Plain HTTP to a host on the LAN is useful for local development only.

**Release** mobile builds combine a stricter **Content-Security-Policy** (see [`src-tauri/tauri.release-csp.json`](../../src-tauri/tauri.release-csp.json) and [tauri-sveltekit.md](../tauri-sveltekit.md)): `connect-src` allows **`https:`** for remote API traffic and **loopback HTTP** (`http://127.0.0.1:*`, `http://localhost:*`) for dev; it does **not** allow arbitrary `http://*` origins. The **Tauri HTTP plugin** capability ([`src-tauri/capabilities/mobile.json`](../../src-tauri/capabilities/mobile.json)) is aligned: **`https://*:*`** plus loopback HTTP patterns only—**not** wide open HTTP to every host.

---

## Base URL and path construction

- Store the server **base URL without a trailing slash** (scheme + host + optional port), e.g. `https://mesh.example.com` or `http://192.168.1.10:3000`.
- The client must build request URLs like the existing **`ServerRequestApi`** (`src/lib/api/shared/server_request_api.ts`, `src/lib/api/web/request.ts`):

  `baseUrl + "/api/v" + version + endpoint`

  with `version` typically `"1"` and `endpoint` starting with `/` (e.g. `/models`, `/login/password`).

---

## Authentication

| Method | Path                     | Notes                                                                                                                             |
| ------ | ------------------------ | --------------------------------------------------------------------------------------------------------------------------------- |
| `POST` | `/api/v1/login/password` | Body: server’s password credentials shape (`PasswordCredentials`). Success: **`204 No Content`**; session established per server. |
| `POST` | `/api/v1/login/token`    | Body: token credentials (`TokenCredentials`). Success: **`204 No Content`**.                                                      |
| `GET`  | `/api/v1/users/me`       | Current user JSON when session valid; **`401`** if not authenticated.                                                             |
| `POST` | `/api/v1/logout`         | End session. Success: **`204 No Content`**.                                                                                       |

### Cookies and `fetch` credentials

The browser-oriented **`ServerRequestApi`** sets `credentials: "same-origin"` on JSON, binary, and multipart requests (`src/lib/api/web/request.ts`). That is correct for same-origin **web** builds.

**Remote mobile** calls a **different origin** (user-configured base URL). For session cookies to be sent and updated, the client stack must:

- use **`credentials: "include"`** (or the **`tauri-plugin-http` equivalent**), **or**
- avoid cookies and use a **token-only** flow aligned with `POST /api/v1/login/token`, if the plugin cannot retain `Set-Cookie`.

Exact behaviour is **TBD** until the auth spike (see **Auth spike outcomes** below).

---

## Existing `/api/v1` routes (inventory)

Sources: `web/src/controller/*_controller.rs`. Path parameters use Axum `{name}` syntax; mobile clients send concrete values.

**Auth (no `login_required` on these four):** see table above.

**Models** (`model_controller`)

| Method   | Path                               |
| -------- | ---------------------------------- |
| `POST`   | `/api/v1/models`                   |
| `GET`    | `/api/v1/models`                   |
| `DELETE` | `/api/v1/models`                   |
| `GET`    | `/api/v1/models/count`             |
| `GET`    | `/api/v1/models/disk_usage`        |
| `PUT`    | `/api/v1/models/{model_id}`        |
| `DELETE` | `/api/v1/models/{model_id}`        |
| `GET`    | `/api/v1/shares/{share_id}/models` |

**Blobs** (`blob_controller`)

| Method | Path                               |
| ------ | ---------------------------------- |
| `GET`  | `/api/v1/models/{model_id}/bytes`  |
| `GET`  | `/api/v1/blobs/{sha256}/bytes`     |
| `POST` | `/api/v1/blobs/download`           |
| `GET`  | `/api/v1/blobs/{sha256}/thumb`     |
| `GET`  | `/api/v1/blobs/{sha256}/download`  |
| `GET`  | `/api/v1/blobs/download/{zip_dir}` |

In the current Axum router, `route_layer(login_required!(Backend))` applies only to routes registered **before** it in this nest; the **thumb**, **single-blob download**, and **`/blobs/download/{zip_dir}`** routes are registered **after** that layer and therefore **do not** use `login_required` in today’s code. Callers should not assume uniform auth on all blob URLs until policy is reviewed.

**Groups** (`group_controller`)

| Method   | Path                               |
| -------- | ---------------------------------- |
| `GET`    | `/api/v1/groups`                   |
| `GET`    | `/api/v1/groups/count`             |
| `POST`   | `/api/v1/groups`                   |
| `DELETE` | `/api/v1/groups/detach_models`     |
| `PUT`    | `/api/v1/groups/{group_id}`        |
| `DELETE` | `/api/v1/groups/{group_id}`        |
| `POST`   | `/api/v1/groups/{group_id}/models` |
| `GET`    | `/api/v1/shares/{share_id}/groups` |

**Labels** (`label_controller`)

| Method   | Path                                 |
| -------- | ------------------------------------ |
| `GET`    | `/api/v1/labels`                     |
| `POST`   | `/api/v1/labels`                     |
| `PUT`    | `/api/v1/labels/{label_id}`          |
| `DELETE` | `/api/v1/labels/{label_id}`          |
| `POST`   | `/api/v1/labels/{label_id}/models`   |
| `DELETE` | `/api/v1/labels/{label_id}/models`   |
| `POST`   | `/api/v1/labels/{label_id}/childs`   |
| `PUT`    | `/api/v1/labels/{label_id}/childs`   |
| `DELETE` | `/api/v1/labels/{label_id}/childs`   |
| `GET`    | `/api/v1/labels/{label_id}/keywords` |
| `PUT`    | `/api/v1/labels/{label_id}/keywords` |
| `PUT`    | `/api/v1/models/{model_id}/labels`   |

**Resources** (`resource_controller`)

| Method   | Path                                     |
| -------- | ---------------------------------------- |
| `GET`    | `/api/v1/resources`                      |
| `POST`   | `/api/v1/resources`                      |
| `PUT`    | `/api/v1/resources/{resource_id}`        |
| `DELETE` | `/api/v1/resources/{resource_id}`        |
| `GET`    | `/api/v1/resources/{resource_id}/groups` |
| `PUT`    | `/api/v1/groups/{group_id}/resource`     |

**Users (admin-style)** (`user_controller`)

| Method   | Path                                  |
| -------- | ------------------------------------- |
| `GET`    | `/api/v1/users`                       |
| `POST`   | `/api/v1/users`                       |
| `PUT`    | `/api/v1/users/{user_id}`             |
| `DELETE` | `/api/v1/users/{user_id}`             |
| `DELETE` | `/api/v1/users/{user_id}/token`       |
| `PUT`    | `/api/v1/users/{user_id}/password`    |
| `PUT`    | `/api/v1/users/{user_id}/permissions` |

**3MF** (`threemf_controller`)

| Method | Path                                     |
| ------ | ---------------------------------------- |
| `GET`  | `/api/v1/models/{model_id}/3mf_metadata` |
| `POST` | `/api/v1/models/{model_id}/3mf_extract`  |

**Shares** (`share_controller`)

| Method   | Path                               |
| -------- | ---------------------------------- |
| `GET`    | `/api/v1/shares`                   |
| `POST`   | `/api/v1/shares`                   |
| `PUT`    | `/api/v1/shares/{share_id}`        |
| `DELETE` | `/api/v1/shares/{share_id}`        |
| `PUT`    | `/api/v1/shares/{share_id}/models` |
| `GET`    | `/api/v1/shares/{share_id}`        |

HTML share pages (`page_controller`, e.g. `/share/{share_id}`) are outside `/api/v1` and are not part of this JSON contract.

---

## New: slicing

**`POST /api/v1/slicer/slice`**

- **Content-Type:** `application/json`
- **Auth:** `login_required` (session); unauthenticated requests receive **401** / **403**.
- **Request:** accepts `modelId` (camelCase) or `model_id` (snake_case). Nested `settings` may use `layerHeight` / `layer_height_mm` and `infill` / `infill_percent` per `web` DTOs (`SliceRequestBody` / `SliceSettingsDto`).
- **Success (200):** JSON body is camelCase:

| Field              | Type             | Meaning                                                                                                           |
| ------------------ | ---------------- | ----------------------------------------------------------------------------------------------------------------- |
| `success`          | `boolean`        | `true` when orchestration completed and the output was registered                                                 |
| `outputBlobId`     | `number`         | **Model id** of the slice output artifact (same value as `SliceOrchestrationResult.output_model_id` in `service`) |
| `outputBlobSha256` | `string`         | Hex SHA-256 of the output blob stored for that model                                                              |
| `message`          | `string \| null` | Optional; omitted from JSON when absent                                                                           |

TypeScript sketch (align clients with the table above):

```typescript
export interface SlicingSettings {
  layerHeight: number;
  infill: number;
  supports: "none" | "everywhere" | "touching buildplate";
  material: string;
}

export interface SliceRequest {
  modelId: number;
  settings: SlicingSettings;
}

export interface SliceResponse {
  success: boolean;
  /** Model id of the registered slice output (not a separate blob-only id). */
  outputBlobId: number;
  outputBlobSha256: string;
  message?: string | null;
}
```

Errors use the normal HTTP error envelope for `web` (not a `SliceResponse` with `success: false`). OrcaSlicer invocation and env requirements are summarized in the appendix below.

---

## Out of scope

Per [§1.2 Non-goals](../superpowers/specs/2026-03-29-remote-client-and-http-api-design.md#12-non-goals-explicit) of the design spec:

- No first-class **`/api/printers`** surface, print job queue, or parallel printer stack in Mesh Organiser v1. Post-slice printing remains **OrcaSlicer** (or operator workflow), not this REST API.

---

## Auth spike outcomes

**Implementation (remote mobile):** `initTauriRemoteApis` uses `ServerRequestApi` with **`credentials: "include"`** and **`fetch` from `@tauri-apps/plugin-http`** (`src/lib/api/tauri/init_remote.ts`, `src/lib/api/web/request.ts`). That matches the **intended** cross-origin session-cookie behaviour; it does **not** replace on-device verification.

- **`tauri-plugin-http` and `Set-Cookie`:** **Verify on Android/iOS** that `POST /api/v1/login/password` (or `…/login/token`) responses that set a session cookie result in **subsequent authenticated** `GET /api/v1/users/me` / model calls without extra client code. If cookies are not retained, switch to **token-only** (`POST /api/v1/login/token`) and document the header/body convention the server expects.
- **Cross-site cookies:** For cookie-based sessions from the app to a **different origin**, the server likely needs **`SameSite=None; Secure`** on the session cookie and **HTTPS** in production; align with `tower-sessions` / Axum cookie settings on `web`.
- **Token fallback:** Same endpoints as today; no change required on the server for token login if cookies fail on device.
- **Client:** Remote path uses **`include`**; browser/web and desktop Tauri local paths keep **`same-origin`** where applicable.

---

## Appendix: server-side slicing (OrcaSlicer / Prusa-family CLI)

**Purpose:** `service::slice_service` runs a **console** slicer on the host for `POST /api/v1/slicer/slice` (Task 3). This appendix documents the **environment variable** and the **v1 subprocess shape**; exact flags may need tuning per install.

| Item                      | Detail                                                                                                                                                                                                                                      |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Executable**            | Set **`MESH_ORGANISER_ORCA_PATH`** to the full path of **OrcaSlicer** or **PrusaSlicer-family** binary. On Windows, prefer **`orca-slicer-console.exe`** (or equivalent) when available so stdout/stderr are usable.                        |
| **v1 invocation**         | `{MESH_ORGANISER_ORCA_PATH} --slice 0 --outputdir <dir> <input_model_path>`                                                                                                                                                                 |
| **Settings JSON**         | Fields such as layer height and infill are modeled in Rust as `SliceOrchestrationSettings` in `service/src/slice_service.rs` for HTTP alignment; **CLI mapping is partial** until flags are verified on a real install (`{binary} --help`). |
| **Further options (TBD)** | Community docs mention `--load-settings`, `--datadir`, `--export-3mf`, etc.; confirm against your OrcaSlicer version before wiring into production.                                                                                         |

If the variable is unset, empty, or the path does not exist, the service returns a clear **`InternalError`** message (no silent failure).

---

_End of contract note — OpenAPI remains optional per phased plan._
