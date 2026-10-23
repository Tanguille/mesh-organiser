# MCP server

The `web` binary serves a [Model Context Protocol](https://modelcontextprotocol.io) endpoint at `/mcp`. It lets an LLM agent such as Claude Code label, rename, group, trash and restore the library of one user.

- Transport: streamable HTTP. It is stateless (no `Mcp-Session-Id`) and answers with JSON, so it works behind a proxy.
- Auth: `Authorization: Bearer <sync token>`. Every tool acts as the user who owns the token. A missing or unknown token gets `401` with `WWW-Authenticate: Bearer`. There is no OAuth: `/.well-known/*` answers `404`.
- Vocabulary: a **mesh** is what the code calls a model, which keeps it apart from LLM models. Mesh ids are the db model ids.
- Code: [`web/src/mcp.rs`](../web/src/mcp.rs).

## Tools

| Tool                                             | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `list_meshes`                                    | Filters by `text_search`, `label_ids`, `group_ids`, `unlabeled_only`, `ungrouped_only`, `printed` and `favorite`. Takes `order_by`, `page` and `page_size` (default 100, max 1000); an unknown `order_by` or an out-of-range page or page size is an error. Returns `total` and each mesh's id, name, group, full label list, printed and favorite flags, file type, size and added date; `label_ids` matches meshes carrying any of the labels directly. |
| `get_meshes`                                     | Full details of up to 50 meshes: link, sha256, timestamps and a plain-text preview of the description (HTML stripped, cut at 2000 characters). For `.3mf` files it adds the first 30 3MF metadata entries (Title, Designer, …), also as previews.                                                                                                                                                                                                         |
| `get_mesh_thumbnails`                            | PNG thumbnails of up to 8 meshes.                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `list_labels`, `list_groups`                     | Every label (color, keywords, parent, children, mesh count) or group (mesh count).                                                                                                                                                                                                                                                                                                                                                                        |
| `create_label`, `update_label`, `delete_label`   | Label CRUD. Keywords are trimmed and lowercased, since imports match them against the lowercased parts of file names. A keyword belongs to one label across all users, so one in use elsewhere fails the call and changes nothing. `delete_label` refuses a label that meshes still carry, trashed ones included.                                                                                                                                         |
| `label_meshes`, `unlabel_meshes`                 | Add a label to meshes or remove it.                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `edit_mesh`                                      | Name, description, link, printed, favorite. The description replaces the stored HTML whole, so an agent must not write `get_meshes`' preview back.                                                                                                                                                                                                                                                                                                        |
| `group_meshes`, `ungroup_meshes`, `rename_group` | Move meshes into an existing or a new group, take them out of their group, or rename a group. An unknown group id is an error.                                                                                                                                                                                                                                                                                                                            |
| `delete_meshes`, `list_trash`, `restore_meshes`  | Delete moves meshes to the trash, which keeps them for `trash_retention_days` (default 30). Restore refuses the whole call if any id is not in the trash.                                                                                                                                                                                                                                                                                                 |
| `get_slicer_link`                                | A `meshorganiser://open_in_slicer?sha256=…` link (see below).                                                                                                                                                                                                                                                                                                                                                                                             |

An argument a tool does not know, such as `labelIds`, is an error rather than ignored. Writes go through the same db/service functions as the REST API and stamp the current time, so desktop sync picks them up.

Mesh descriptions and 3MF metadata come from model sites. The server instructions tell the agent to treat them as data, never as instructions.

## Getting the token

The token is the user's **sync token**, the same one that links a desktop app to the account. Rotating it ("Reset linked desktop instances" in the settings) disconnects both linked desktops and MCP clients.

Two ways to read it:

- **Browser.** Log in to the web UI and open `/api/v1/users/me` in the same browser. The token is the `sync_token` field. If it is `null`, use "Link account to desktop Mesh Organiser" from the user menu once to create one.
- **curl.** The session cookie is `Secure`, so this works over HTTPS or on `localhost`. Use the account that owns the library, not the local administrator (`local@noemail.com`), which the web UI refuses for normal use.

  ```bash
  base=https://mesh.example.com
  curl -c jar -X POST "$base/api/v1/login/password" -H 'Content-Type: application/json' \
      -d '{"email":"<email>","password":"<password>"}'
  curl -b jar "$base/api/v1/users/me" | jq '{id, sync_token}'
  # sync_token is null: create one (this also rotates an existing one), then read it again
  curl -b jar -X DELETE "$base/api/v1/users/<id>/token"
  ```

## Claude Code

```bash
claude mcp add --transport http mesh-organiser http://<host>:<port>/mcp \
    --header "Authorization: Bearer <token>"
```

## Kubernetes with ToolHive

This is an example to adapt, not a tested manifest. ToolHive fronts the in-cluster service with its own OIDC auth and injects the sync token, so clients never see the token:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: mesh-organiser-mcp
  namespace: toolhive
stringData:
  token: <sync token>
---
apiVersion: toolhive.stacklok.dev/v1beta1
kind: MCPExternalAuthConfig
metadata:
  name: mesh-organiser-token
  namespace: toolhive
spec:
  type: bearerToken
  bearerToken:
    tokenSecretRef:
      name: mesh-organiser-mcp
      key: token
---
apiVersion: toolhive.stacklok.dev/v1beta1
kind: MCPRemoteProxy
metadata:
  name: mesh-organiser
  namespace: toolhive
spec:
  remoteUrl: http://mesh-organiser.mesh-organiser.svc.cluster.local:3000/mcp
  # The remote is a cluster-internal address.
  allowPrivateEndpoint: true
  transport: streamable-http
  externalAuthConfigRef:
    name: mesh-organiser-token
  oidcConfigRef:
    name: <your-oidc-config> # placeholder: how clients authenticate to the proxy
    # Required, and unique per proxied server: the token audience the proxy accepts.
    audience: mesh-organiser
```

### Deployment notes

- **Keep `/mcp` off the public ingress.** The sync token is the only thing that guards it, and it grants full access to the account. Route `/mcp` only inside the cluster, through the proxy above.
- **Run one replica.** SQLite allows a single writer, and the import lock and the hourly trash purge run inside the process.
- **Put SQLite on a block-backed `ReadWriteOnce` PVC**, not on NFS or SMB. SQLite's file locking is unreliable on network filesystems, which corrupts the database.

## Opening meshes in the slicer

The server cannot reach the desktop the agent runs on, so it hands the agent a link instead:

1. The agent calls `get_slicer_link(mesh_ids)` and gets `meshorganiser://open_in_slicer?sha256=<sha256>[,<sha256>...]`.
2. On the desktop that runs Mesh Organiser, the agent opens the link with the OS opener: `xdg-open '<link>'` on Linux, `open '<link>'` on macOS, `start "" "<link>"` on Windows.
3. The desktop app resolves the meshes by sha256 and opens them in its configured slicer. An online-only account downloads them first. A local account must already have them, for example through sync.
