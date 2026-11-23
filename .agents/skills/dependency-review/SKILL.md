---
name: dependency-review
description: Repo-specific facts for reviewing Renovate dependency bumps in mesh-organiser. Load on Renovate PRs.
---

# Dependency review

## Where dependencies are used

- npm: one root `package.json`, no pnpm workspace. `vite.config.js` `optimizeDeps.include` names deep imports of `three`, `@lucide/svelte`, `@tauri-apps/*` and `bits-ui`. Check that a bump keeps each entry point.
- Cargo: versions live in root `Cargo.toml` `[workspace.dependencies]`. Members are `db`, `libmeshthumbnail`, `service`, `web` and `src-tauri`. Test-only crates (`hyper`, `tokio-rustls`, `rcgen`) are versioned in `web/Cargo.toml`.
- `src-tauri` is the desktop app. `web` is the server that the `Dockerfile` builds with `cargo build -p web --locked`.
- `mise.toml` pins Node for mise and the Dockerfile. Mise and frontend CI read the pnpm pin from `package.json` `packageManager`, which must match the lockfile.
- Keep the Dockerfile builder `rust:<version>-slim-trixie` and the runtime `debian:trixie-slim` on one Debian release.
- Keep every GitHub Action on a full SHA with its version comment.

## Versions that move together

- `three` and `@types/three`.
- `@sveltejs/kit` and `@sveltejs/adapter-static`.
- `libsqlite3-sys` and the copy `sqlx` pulls in: `Cargo.lock` must hold one, and `db/src/sync_transport/stage_slots.rs` calls it directly.
- `cadrum`: `scripts/fetch-occt.sh` reads `OCCT_VERSION` from its `build.rs`, so a renamed constant fails CI.

## What to run

- Frontend, as in svelte-ci: `pnpm run lint`, `format:check`, `check`, `test`, `build`.
- The Docker image builds the frontend with `VITE_API_PLATFORM=web`, which CI never does. Check web-only code paths for Svelte, Vite and `three` bumps.
- Rust, as in rust-ci: `cargo fmt --all -- --check`, cargo-machete, `cargo clippy --workspace --all-targets`, `cargo test --workspace --all-targets`. Linux compiles OCCT from source, about 30 minutes.
- Thumbnails come from `libmeshthumbnail`, which renders with `euc` pinned by git rev. Run `cargo test -p libmeshthumbnail` for `image`, `stl_io`, `threemf`, `wavefront_obj` and `cadrum` bumps.
- Every shipped npm or cargo update, majors included, is titled `deps(<manager>):` and bumps the app patch version. Flag one titled `feat`, `fix` or with `!`: the title rule did not apply and the PR would cut a minor or major release.
