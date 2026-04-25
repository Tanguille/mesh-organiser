# Publishing a release

This repo uses GitHub Actions to build the Tauri desktop app. You can create a public release, a draft release, or just build artifacts without creating a release.

## What you need to do

### 1. Version in one place

The app version is defined in **`package.json`** (`version`) and **must match** `Cargo.toml` `[workspace.package]` `version`. **`src-tauri/tauri.conf.json`** must keep `"version": "../package.json"` so Tauri uses that same value (the publish workflow replaces `__VERSION__` from this resolved version, e.g. tag `v1.0.0`).

**CI enforces this:** `node scripts/check-package-cargo-version.mjs` runs on **Svelte CI** and **before every publish matrix build**. It fails if `package.json`, Cargo workspace, or the Tauri `version` pointer are inconsistent.

- Before releasing, bump the version in **`package.json`** and **`Cargo.toml`** `[workspace.package]` together (or run the check after editing).
- If you use **tag-triggered** releases (see below), the tag you push (e.g. `v2.7.0`) should match the semver in `package.json` (e.g. `2.7.0`).

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

- Set the version in `package.json` and `Cargo.toml` (e.g. `2.7.0`), with `tauri.conf.json` still pointing at `../package.json`.
- Commit, push, then create and push a tag that matches:
  `git tag v2.7.0 && git push origin v2.7.0`
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
