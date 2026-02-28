# Publishing a release

This repo uses GitHub Actions to build the Tauri desktop app and attach artifacts to a **draft** GitHub release. You then publish the draft when ready.

## What you need to do

### 1. Version in one place

The release **tag and name** use the version from **`src-tauri/tauri.conf.json`** (`version`). The Tauri action replaces `__VERSION__` with that value (e.g. `1.0.0` → tag `v1.0.0`).

- Before releasing, set the version you want in `src-tauri/tauri.conf.json`.
- If you use **tag-triggered** releases (see below), the tag you push (e.g. `v1.0.0`) should match that version.

### 2. Optional: updater signing (recommended for public releases)

For signed in-app updates you need two repository **Secrets** (Settings → Secrets and variables → Actions):

| Secret               | Description                              |
|----------------------|------------------------------------------|
| `TAURI_PRIVATE_KEY`  | Private key from `tauri signer generate` |
| `TAURI_KEY_PASSWORD` | Password for that key                    |

Without these, builds and releases still work; only the updater signature is skipped.

### 3. Run the release workflow

Tag-triggered

- Set the version in `src-tauri/tauri.conf.json` (e.g. `1.0.0`).
- Commit, push, then create and push a tag that matches:
  `git tag v1.0.0 && git push origin v1.0.0`
- The **publish** workflow runs and creates/updates a draft release for that tag. Publish the draft when ready.

### 4. After the run

- Find the draft release under **Releases**.
- Confirm all expected platform assets (Windows, Linux, macOS) are attached.
- Click **Publish release** to make it public.

## Workflows

- **publish** (`release.yaml`) – Builds the app and creates/updates a **draft** release with artifacts. Trigger: manual or push of tag `v*`.
- **force-build** (`force-build.yml`) – Builds only and uploads artifacts as workflow artifacts (no release). Use for testing the build pipeline.

## Notes

- The first run can be slow; later runs use Rust and pnpm caches.
- `pnpm install --frozen-lockfile` is used in CI; ensure `pnpm-lock.yaml` is committed and up to date.
- All third-party actions are pinned by full commit SHA (see workflow files); update the SHAs periodically for security and latest fixes.
