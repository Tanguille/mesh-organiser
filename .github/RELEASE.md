# Publishing a release

This repo uses GitHub Actions to build the Tauri desktop app. You can create a public release, a draft release, or just build artifacts without creating a release.

## What you need to do

### 1. Version in one place

The release **tag and name** use the version from **`src-tauri/tauri.conf.json`** (`version`). The Tauri action replaces `__VERSION__` with that value (e.g. `1.0.0` → tag `v1.0.0`).

- Before releasing, set the version you want in `src-tauri/tauri.conf.json`.
- If you use **tag-triggered** releases (see below), the tag you push (e.g. `v1.0.0`) should match that version.

### 2. Optional: updater signing (recommended for public releases)

For signed in-app updates you need two repository **Secrets** (Settings → Secrets and variables → Actions):

| Secret               | Description                              |
| -------------------- | ---------------------------------------- |
| `TAURI_PRIVATE_KEY`  | Private key from `tauri signer generate` |
| `TAURI_KEY_PASSWORD` | Password for that key                    |

Without these, builds and releases still work; only the updater signature is skipped.

### 3. Run the release workflow

#### Option A: Manual dispatch (recommended)

1. Go to **Actions** → **publish** workflow → **Run workflow**
2. Choose **Release type**:
   - **publish** – Creates a public release immediately
   - **draft** – Creates a draft release for review before publishing
   - **build-only** – Builds artifacts only, no release created (artifacts available in Actions run)
3. Click **Run workflow**

#### Option B: Tag-triggered

- Set the version in `src-tauri/tauri.conf.json` (e.g. `1.0.0`).
- Commit, push, then create and push a tag that matches:
  `git tag v1.0.0 && git push origin v1.0.0`
- This creates a **public** release automatically.

### 4. After the run

For draft releases:

- Find the draft release under **Releases**.
- Confirm all expected platform assets (Windows, Linux, macOS) are attached.
- Click **Publish release** to make it public.

For build-only:

- Download artifacts from the **Actions** run page (under "Artifacts").

## Workflows

- **publish** (`release.yaml`) – Builds the app and creates a release or artifacts. Trigger: manual (workflow_dispatch) or push of tag `v*`.

## Notes

- The first run can be slow; later runs use Rust and pnpm caches.
- `pnpm install --frozen-lockfile` is used in CI; ensure `pnpm-lock.yaml` is committed and up to date.
- All third-party actions are pinned by full commit SHA (see workflow files); update the SHAs periodically for security and latest fixes.
