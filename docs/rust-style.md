# Rust Backend Code Style

## General Conventions
- Follow standard Rust conventions (rustfmt auto-formats)
- Use `#[tauri::command]` for Tauri command handlers
- Use sqlx for database queries
- Enable clippy lints (checked in CI)

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
