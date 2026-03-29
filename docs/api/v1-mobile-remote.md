# HTTP API — v1 mobile remote client

Contract note for **Tauri mobile** talking to a remotely hosted **`web`** (Axum) instance. Canonical product decisions live in [2026-03-29-remote-client-and-http-api-design.md](../superpowers/specs/2026-03-29-remote-client-and-http-api-design.md).

---

## Base URL and path construction

- Store the server **base URL without a trailing slash** (scheme + host + optional port), e.g. `https://mesh.example.com` or `http://192.168.1.10:3000`.
- The client must build request URLs like the existing **`ServerRequestApi`** (`src/lib/api/shared/server_request_api.ts`, `src/lib/api/web/request.ts`):

  `baseUrl + "/api/v" + version + endpoint`

  with `version` typically `"1"` and `endpoint` starting with `/` (e.g. `/models`, `/login/password`).

---

## Authentication

| Method | Path | Notes |
|--------|------|--------|
| `POST` | `/api/v1/login/password` | Body: server’s password credentials shape (`PasswordCredentials`). Success: **`204 No Content`**; session established per server. |
| `POST` | `/api/v1/login/token` | Body: token credentials (`TokenCredentials`). Success: **`204 No Content`**. |
| `GET` | `/api/v1/users/me` | Current user JSON when session valid; **`401`** if not authenticated. |
| `POST` | `/api/v1/logout` | End session. Success: **`204 No Content`**. |

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

| Method | Path |
|--------|------|
| `POST` | `/api/v1/models` |
| `GET` | `/api/v1/models` |
| `DELETE` | `/api/v1/models` |
| `GET` | `/api/v1/models/count` |
| `GET` | `/api/v1/models/disk_usage` |
| `PUT` | `/api/v1/models/{model_id}` |
| `DELETE` | `/api/v1/models/{model_id}` |
| `GET` | `/api/v1/shares/{share_id}/models` |

**Blobs** (`blob_controller`)

| Method | Path |
|--------|------|
| `GET` | `/api/v1/models/{model_id}/bytes` |
| `GET` | `/api/v1/blobs/{sha256}/bytes` |
| `POST` | `/api/v1/blobs/download` |
| `GET` | `/api/v1/blobs/{sha256}/thumb` |
| `GET` | `/api/v1/blobs/{sha256}/download` |
| `GET` | `/api/v1/blobs/download/{zip_dir}` |

In the current Axum router, `route_layer(login_required!(Backend))` applies only to routes registered **before** it in this nest; the **thumb**, **single-blob download**, and **`/blobs/download/{zip_dir}`** routes are registered **after** that layer and therefore **do not** use `login_required` in today’s code. Callers should not assume uniform auth on all blob URLs until policy is reviewed.

**Groups** (`group_controller`)

| Method | Path |
|--------|------|
| `GET` | `/api/v1/groups` |
| `GET` | `/api/v1/groups/count` |
| `POST` | `/api/v1/groups` |
| `DELETE` | `/api/v1/groups/detach_models` |
| `PUT` | `/api/v1/groups/{group_id}` |
| `DELETE` | `/api/v1/groups/{group_id}` |
| `POST` | `/api/v1/groups/{group_id}/models` |
| `GET` | `/api/v1/shares/{share_id}/groups` |

**Labels** (`label_controller`)

| Method | Path |
|--------|------|
| `GET` | `/api/v1/labels` |
| `POST` | `/api/v1/labels` |
| `PUT` | `/api/v1/labels/{label_id}` |
| `DELETE` | `/api/v1/labels/{label_id}` |
| `POST` | `/api/v1/labels/{label_id}/models` |
| `DELETE` | `/api/v1/labels/{label_id}/models` |
| `POST` | `/api/v1/labels/{label_id}/childs` |
| `PUT` | `/api/v1/labels/{label_id}/childs` |
| `DELETE` | `/api/v1/labels/{label_id}/childs` |
| `GET` | `/api/v1/labels/{label_id}/keywords` |
| `PUT` | `/api/v1/labels/{label_id}/keywords` |
| `PUT` | `/api/v1/models/{model_id}/labels` |

**Resources** (`resource_controller`)

| Method | Path |
|--------|------|
| `GET` | `/api/v1/resources` |
| `POST` | `/api/v1/resources` |
| `PUT` | `/api/v1/resources/{resource_id}` |
| `DELETE` | `/api/v1/resources/{resource_id}` |
| `GET` | `/api/v1/resources/{resource_id}/groups` |
| `PUT` | `/api/v1/groups/{group_id}/resource` |

**Users (admin-style)** (`user_controller`)

| Method | Path |
|--------|------|
| `GET` | `/api/v1/users` |
| `POST` | `/api/v1/users` |
| `PUT` | `/api/v1/users/{user_id}` |
| `DELETE` | `/api/v1/users/{user_id}` |
| `DELETE` | `/api/v1/users/{user_id}/token` |
| `PUT` | `/api/v1/users/{user_id}/password` |
| `PUT` | `/api/v1/users/{user_id}/permissions` |

**3MF** (`threemf_controller`)

| Method | Path |
|--------|------|
| `GET` | `/api/v1/models/{model_id}/3mf_metadata` |
| `POST` | `/api/v1/models/{model_id}/3mf_extract` |

**Shares** (`share_controller`)

| Method | Path |
|--------|------|
| `GET` | `/api/v1/shares` |
| `POST` | `/api/v1/shares` |
| `PUT` | `/api/v1/shares/{share_id}` |
| `DELETE` | `/api/v1/shares/{share_id}` |
| `PUT` | `/api/v1/shares/{share_id}/models` |
| `GET` | `/api/v1/shares/{share_id}` |

HTML share pages (`page_controller`, e.g. `/share/{share_id}`) are outside `/api/v1` and are not part of this JSON contract.

---

## New: slicing

**`POST /api/v1/slicer/slice`**

- **Status:** specified here for contract-first delivery; **not yet implemented** in `web` at the time this note was added.
- **Content-Type:** `application/json`
- **Body / response:** align TypeScript and future Rust DTOs with:

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
  outputBlobId: number | null;
  message: string | null;
}
```

Server behaviour (OrcaSlicer invocation, error mapping, auth) will be defined in implementation; this note fixes the **JSON shapes** for client and server alignment.

---

## Out of scope

Per [§1.2 Non-goals](../superpowers/specs/2026-03-29-remote-client-and-http-api-design.md#12-non-goals-explicit) of the design spec:

- No first-class **`/api/printers`** surface, print job queue, or parallel printer stack in Mesh Organiser v1. Post-slice printing remains **OrcaSlicer** (or operator workflow), not this REST API.

---

## Auth spike outcomes

**TBD until spike.** Fill this in before shipping **Task 7** (remote `initApi` / `ServerRequestApi` and fetch options) in [2026-03-29-remote-client-http-api.md](../superpowers/plans/2026-03-29-remote-client-http-api.md).

- **`tauri-plugin-http` and `Set-Cookie`:** whether responses from `POST /api/v1/login/password` / `…/login/token` result in a **stored session cookie** on device, or whether the plugin ignores or drops `Set-Cookie` (**unverified**).
- **Cross-site cookies:** if cookies are used from the mobile app origin to the server origin, whether the server must emit **`SameSite=None; Secure`** (and HTTPS) for the session cookie to persist (**unverified**).
- **Token fallback:** if cookies are not viable, document **token-only** auth (`POST /api/v1/login/token` + header or body convention the server accepts) as the supported mobile path (**unverified**).
- **Client changes:** link outcomes to edits under **`src/lib/api/web/request.ts`** (e.g. configurable `credentials`, or a dedicated remote fetch wrapper used by **Task 7** instead of `same-origin` for mobile).

---

_End of contract note — OpenAPI remains optional per phased plan._
