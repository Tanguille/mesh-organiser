# Rust Backend Code Style

## General Conventions

- Follow standard Rust conventions (rustfmt auto-formats)
- Use `#[tauri::command]` for Tauri command handlers
- Use sqlx for database queries
- Enable clippy lints (checked in CI)
- In `format!` / `format_args!`, prefer **inline names in braces** (`{var}`) when the value is already in a variable, so the string is self-explanatory. Example: `format!("IN ({placeholders})")` rather than `format!("IN ({})", placeholders)`. Do not introduce a variable only to use it in braces; keep the argument form when the value is an expression (e.g. `format!("IN ({})", join(ids, ","))`).

## Imports

Where multiple items are imported from the same path, use a single `use` statement with a braced list instead of multiple separate lines (one `use` per path prefix).

**Good:**

```rust
use std::path::{Path, PathBuf};
use service::{AppState, Configuration};
```

**Avoid:**

```rust
use std::path::Path;
use std::path::PathBuf;
use service::AppState;
use service::Configuration;
```

`cargo fmt` does not merge imports; this is a project convention. For very long braced lists (e.g. 10+ items), split by logical subpath for readability.

### Order

List `use` statements in this order (top to bottom), with a single blank line between groups. Omit any group that does not appear in the file.

1. **std** – standard library (`use std::...`)
2. **External** – third-party / crates.io crates (e.g. `serde`, `tokio`, `axum`, `tauri`)
3. **Crates** – other workspace member crates (`db`, `service`)
4. **crate** – current crate (`use crate::...`)
5. **super** – parent module (`use super::...`)

Example (one `use` per path, single `use crate::...` with braced list):

```rust
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use db::model_db;
use service::Configuration;

use crate::{error::ApplicationError, tauri_app_state::TauriAppState};

use super::app_state::AppState;
```

### Module declarations

Follow the [Rust style guide](https://doc.rust-lang.org/beta/style-guide/items.html): `use` statements and module declarations (`mod foo;`) must come before other items. Put **imports before module declarations**.

- In each file: first all `use` statements (in the [import order](#order) above), then all `mod` declarations, then the rest of the file (types, functions, re-exports, etc.).
- Sort module declarations **alphabetically** (version-sort). Keep `#[cfg(...)]` on the same line as the `mod` it applies to; sort only by the module name when comparing.
- **Feature-gated items** follow the same order: cfg-gated `use` in the use section, cfg-gated `mod` in the mod section, cfg-gated `pub use` or code in the rest. Do not group by feature at the expense of use-before-mod.
- Do not move or reorder `#[macro_use]` or other attribute-annotated mods if that could change semantics.

### Module file layout

Prefer the **module-name-as-filename** layout over top-level `mod.rs` files ([Rust book](https://doc.rust-lang.org/book/ch07-05-separating-modules-into-different-files.html)):

- For a module `foo`, use **`foo.rs`** (same directory as the parent), not **`foo/mod.rs`**.
- Submodules of `foo` live in the **`foo/`** directory (e.g. `foo/bar.rs` when `foo` contains `mod bar;`). The compiler looks up child modules there, not as siblings of `foo.rs`.

This avoids many files named `mod.rs` and keeps the module tree easier to follow. If two sibling modules would both need a submodule with the same name (e.g. two `gcode` submodules in different parents), keep one of them as a directory with `mod.rs` or rename the submodule to avoid the collision.

Example:

- **Prefer:** `src/api.rs` and `src/api/blob_api.rs`, `src/api/model_api.rs`, …
- **Avoid:** `src/api/mod.rs` and `src/api/blob_api.rs`, … (same children, but parent is the redundant `mod.rs`).

## Error Handling

Use `ApplicationError` enum with `?` operator for propagation:

```rust
#[tauri::command]
async fn get_models(state: State<'_, TauriAppState>) -> Result<Vec<Model>, ApplicationError> {
    let models = model_db::get_all_models(&state.app_state.db).await?;
    Ok(models)
}
```

## Commands

Run from repository root for entire workspace:

```bash
# Check formatting
cargo fmt --all -- --check

# Lint (builds implicitly)
cargo clippy --workspace --all-targets --all-features

# Build
cargo build --workspace
```
