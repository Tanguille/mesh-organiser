# Rust Backend Code Style

## General Conventions

- Follow standard Rust conventions (rustfmt auto-formats)
- Use `#[tauri::command]` for Tauri command handlers
- Use sqlx for database queries
- Enable clippy lints (checked in CI)
- In `format!` / `format_args!`, prefer **inline names in braces** (`{var}`) when the value is already in a variable, so the string is self-explanatory. Example: `format!("IN ({placeholders})")` rather than `format!("IN ({})", placeholders)`. Do not introduce a variable only to use it in braces; keep the argument form when the value is an expression (e.g. `format!("IN ({})", join(ids, ","))`).

### Identifiers

- Prefer **full words** (`model_id`, `total`, `user`, `chunk`) over single-letter names.
- **Exception:** `e` for an error value in `if let Err(e)`, `Result::map_err(|e| ...)`, and similar is fine.
- **Narrow exception:** trivial **loop indices** in very short loops (`for i in …`, `enumerate` with `i` / `j`) when conventional; still prefer a word if the index carries meaning beyond position.
- Avoid one-letter names for domain data (`n`, `x`, `t`, `m`) unless a longer name would be genuinely unwieldy (rare).

## Control flow and whitespace

- Prefer a **blank line** between preceding logic and a **`return`** (including `return Ok(...)`, `return Err(...)`, or early exit from a branch), so the exit path is easy to scan. Apply the same spacing before the **final expression** in a function when it follows a multi-line or non-trivial block (even if you omit the `return` keyword).
- Omit the extra line when it would hurt readability (e.g. a one-line guard `if !ready { return; }` at the top of a function, or a very short `match` arm).

**Good:**

```rust
fn sum_ids(ids: &[u64]) -> u64 {
    let mut total = 0;
    for &id in ids {
        total += id;
    }

    total
}
```

## Types and invariants (“tight typing”)

Prefer types that match the **domain**: if a value cannot meaningfully be negative, prefer an **unsigned** integer (`u32`, `u64`, `usize`) or a **newtype** / wrapper that enforces validation at construction.

- **Counts, lengths, sizes, non-negative offsets**: prefer `usize` for indexing and in-memory sizes; `u32`/`u64` for stable wire or storage sizes when the domain is non-negative.
- **Signed types** (`i32`, `i64`): use when the domain includes negative values, optional foreign keys, or API/schema compatibility (e.g. SQLite `INTEGER`, serde from JSON) — document why if it looks “loose”.
- **Boundaries**: external formats (DB columns, HTTP query params, serde) often fix the type; validate at the edge and convert explicitly rather than scattering `as u64` / silent casts. Watch for `clippy::cast_possible_wrap` / sign-loss when mixing signed and unsigned.

For critical ranges (e.g. “must be 1..=100”), consider a **small type with a constructor** (`fn new(x: i32) -> Option<Self>`) so call sites cannot hold an invalid value ([Rust book: custom types for validation](https://doc.rust-lang.org/book/ch09-03-to-panic-or-not-to-panic.html#custom-types-for-validation)).

**Existing code:** tighten types in **new** code and at **clear boundaries** (new endpoints, DB mappers). Avoid unrelated mass migrations of `i64`/`i32` across the codebase in a single change set.

## Strings

Rust’s main UTF-8 text types are **`String`** (owned), **`&str`** (borrowed), and **`Cow<'_, str>`** when you may borrow or allocate. See the [Rust book on strings](https://doc.rust-lang.org/book/ch08-02-strings.html) for indexing and UTF-8 pitfalls.

### Choosing parameter types

| Situation                                                                                                            | Prefer                                                                                                                                                       |
| -------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Read-only, borrowed text in **most** internal helpers                                                                | **`&str`** — simplest; `&String` and `&str` both coerce for callers ([`Deref`](https://doc.rust-lang.org/std/ops/trait.Deref.html) to `str`).                |
| **Public / shared** API that should accept `String`, `&str`, `Cow<str>`, etc. without awkward `&` at every call site | **`impl AsRef<str>`** — call `s.as_ref()` once in the body. See [`AsRef`](https://doc.rust-lang.org/std/convert/trait.AsRef.html).                           |
| Multiple string parameters each needing `AsRef<str>`                                                                 | Separate type parameters, e.g. `S1: AsRef<str>, S2: AsRef<str>`, or use `&str` if callers can pass `&` — otherwise the compiler may unify one type for both. |
| **Own** or **consume** the text (store, mutate, move on)                                                             | **`String`** or **`impl Into<String>`** (accepts `String` and `&str` with allocation cost for the latter).                                                   |
| Sometimes borrow, sometimes allocate (e.g. optional default)                                                         | **`Cow<'_, str>`**.                                                                                                                                          |

**`impl AsRef<str>` is not the default everywhere:** it adds generic noise and monomorphization; many codebases use **`&str`** for most functions and reserve **`AsRef<str>`** for crate boundaries or helpers called with mixed owned/borrowed strings. Use whichever matches call sites without extra ceremony.

### Paths

Use **`&Path`**, **`PathBuf`**, or **`impl AsRef<Path>`** for filesystem paths — not `AsRef<str>` (paths are not always valid UTF-8 on all platforms). See [`std::path::Path`](https://doc.rust-lang.org/std/path/struct.Path.html).

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
- **Tauri:** `src/commands.rs` with `src/commands/server_url.rs` (and further submodules under `commands/`), not `src/commands/mod.rs`.

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

The workspace enables **`clippy::pedantic`** (including cast lints such as `cast_possible_truncation`, `cast_possible_wrap`, `cast_sign_loss`), **`clippy::nursery`**, **`clippy::style`**, plus `unnecessary_self_imports` (restriction; see root `Cargo.toml` `[workspace.lints.clippy]`). Prefer explicit conversions (`try_from`, `checked_*`, `unsigned_abs`) or a targeted `#[allow(...)]` with a one-line reason when a cast is intentional.
