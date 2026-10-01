# Publishing a release

This repo uses GitHub Actions to build the Tauri desktop app. You can create a public release, a draft release, or just build artifacts without creating a release.

## Automatic release PRs (release-please)

Pushes to `main` run the **release-please** workflow. It reads
[Conventional Commits](https://www.conventionalcommits.org/) since the last
release (`fix:` → patch, `feat:` → minor, breaking `!` → major) and keeps a
release PR up to date. The PR bumps `package.json` and `Cargo.toml` together
plus `CHANGELOG.md`; merging it triggers `release-tag` below, which tags,
creates the GitHub Release, and dispatches `publish`. No manual version bump
or tag push is needed. (`Cargo.lock` catches up on the next `cargo` build;
nothing in CI uses `--locked`.)

Config: `release-please-config.json` + `.release-please-manifest.json`.
Tagging stays with `release-tag` (`skip-github-release: true`).

> Recommended: add a `RELEASE_PLEASE_TOKEN` secret (PAT with `repo` scope) so
> CI runs on release-please PRs — `GITHUB_TOKEN`-created PRs don't trigger
> other workflows. Falls back to `GITHUB_TOKEN` when absent.

## What you need to do (manual fallback)

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

Merging a release PR (e.g. `release/v4.0.0` → `main`) creates the release
automatically: the `release-tag` workflow tags the new `package.json` version
(`v4.0.0`) and opens the GitHub Release with auto-generated notes
(PRs/commits since the previous tag), and the tag push triggers `publish`
to build the binaries. No manual tag push is needed.

#### Option A: Manual dispatch (recommended for drafts / rebuilds)

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

- **release-please** (`release-please.yaml`) – Opens/updates the release PR on push to `main`. Merge it to release.
- **release-tag** (`release-tag.yaml`) – Auto-tags `v<package.json version>` and creates the GitHub Release on push to `main`. Trigger: push to `main` touching `package.json`/`Cargo.toml`.
- **publish** (`release.yaml`) – Builds the app and creates a release or artifacts. Trigger: manual (workflow_dispatch) or push of tag `v*`.

## Notes

- The first run can be slow; later runs use Rust and pnpm caches.
- `pnpm install --frozen-lockfile` is used in CI; ensure `pnpm-lock.yaml` is committed and up to date.
- All third-party actions are pinned by full commit SHA (see workflow files); update the SHAs periodically for security and latest fixes.
