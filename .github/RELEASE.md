# Publishing a release

GitHub Actions builds the Tauri desktop app and publishes it as a public or draft release. Public releases (tag push or **publish**) also push the web image to `ghcr.io/tanguille/mesh-organiser` as `latest` and `<tag>`; **draft** and **build-only** runs skip it.

## Automatic release PRs (release-please)

Pushes to `main` run the **release-please** workflow. It reads
[Conventional Commits](https://www.conventionalcommits.org/) since the last
release (`fix:` → patch, `feat:` → minor, breaking `!` → major) and keeps a
release PR up to date. The PR bumps `package.json`, `Cargo.toml` and the
workspace crates in `Cargo.lock` (every `[[package]]` without a `source`)
together plus `CHANGELOG.md`. Merging it triggers **release-tag**, which tags
`v<version>`, creates the GitHub Release with auto-generated notes
(PRs/commits since the previous tag), and dispatches **publish** to build and
attach the binaries. No manual version bump or tag push is needed.

Config: `release-please-config.json` + `.release-please-manifest.json`.

> Recommended: add a `RELEASE_PLEASE_TOKEN` secret (PAT with `repo` scope) so
> CI runs on release-please PRs. Falls back to `GITHUB_TOKEN` when absent.

## Version in one place

The app version is **`package.json`** `version` and **must match** `Cargo.toml` `[workspace.package]` `version`. **`src-tauri/tauri.conf.json`** must keep `"version": "../package.json"` so Tauri uses that same value (the publish workflow replaces `__VERSION__` with it, e.g. tag `v1.0.0`).

`node scripts/check-package-cargo-version.mjs` runs before every publish matrix build and fails if `package.json`, the Cargo workspace, or the Tauri `version` pointer are inconsistent.

## Updater signing (optional, recommended for public releases)

For signed in-app updates you need two repository **Secrets** (Settings → Secrets and variables → Actions):

| Secret               | Description                              |
| -------------------- | ---------------------------------------- |
| `TAURI_PRIVATE_KEY`  | Private key from `tauri signer generate` |
| `TAURI_KEY_PASSWORD` | Password for that key                    |

Without these, builds and releases still work; only the updater signature is skipped.

## Manual runs (drafts, rebuilds)

1. Go to **Actions** → **publish** workflow → **Run workflow**
2. Choose **Release type**:
   - **publish** – Creates a public release immediately
   - **draft** – Creates a draft release for review before publishing
   - **build-only** – Builds every platform without creating a release or uploading artifacts
3. Click **Run workflow**

Pushing a tag that matches `package.json` (`git tag v2.7.0 && git push origin v2.7.0`) also builds a **public** release.

For draft releases, find the draft under **Releases**, confirm all expected platform assets (Windows, Linux, macOS) are attached, then click **Publish release**.

## Notes

- The first run can be slow; later runs use Rust and pnpm caches.
- `pnpm install --frozen-lockfile` is used in CI; ensure `pnpm-lock.yaml` is committed and up to date.
- All third-party actions are pinned by full commit SHA (see workflow files); update the SHAs periodically for security and latest fixes.
