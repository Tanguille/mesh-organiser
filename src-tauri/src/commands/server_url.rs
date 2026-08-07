//! Persist the remote mesh-organiser server base URL (mobile).

use tauri::AppHandle;
use tauri_plugin_store::{JsonValue, StoreExt};

/// Key for the stored base URL in the plugin store file.
pub const STORE_KEY: &str = "remote_server_url";

const STORE_FILE: &str = "remote_client_store.json";

fn normalize_remote_server_url(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(String::from("URL cannot be empty"));
    }

    if trimmed.chars().any(char::is_control) {
        return Err(String::from("URL must not contain control characters"));
    }

    let without_trailing = trimmed.trim_end_matches('/');
    let scheme_len = if without_trailing
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
    {
        8usize
    } else if without_trailing
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
    {
        7usize
    } else {
        return Err(String::from("URL must start with http:// or https://"));
    };

    let after_scheme = without_trailing
        .get(scheme_len..)
        .ok_or_else(|| String::from("URL must include a host after the scheme"))?;

    if after_scheme.chars().any(char::is_whitespace) {
        return Err(String::from(
            "URL must not contain whitespace in the host or path",
        ));
    }

    if after_scheme.is_empty() || after_scheme.starts_with('/') {
        return Err(String::from(
            "URL must include a host (e.g. https://example.com)",
        ));
    }

    Ok(without_trailing.to_string())
}

fn store_string_value(value: &JsonValue) -> Result<String, String> {
    value
        .as_str()
        .map(ToString::to_string)
        .ok_or_else(|| String::from("stored value is not a string"))
}

#[tauri::command]
#[allow(clippy::missing_errors_doc)]
pub async fn get_server_url(app: AppHandle) -> Result<Option<String>, String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;

    let Some(raw) = store.get(STORE_KEY) else {
        return Ok(None);
    };

    Ok(Some(store_string_value(&raw)?))
}

#[tauri::command]
#[allow(clippy::missing_errors_doc)]
pub async fn set_server_url(app: AppHandle, url: String) -> Result<(), String> {
    let normalized = normalize_remote_server_url(&url)?;

    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;

    store.set(STORE_KEY, JsonValue::from(normalized));
    store.save().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[allow(clippy::missing_errors_doc)]
pub async fn clear_server_url(app: AppHandle) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;

    let _removed = store.delete(STORE_KEY);
    store.save().map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize_remote_server_url;

    #[test]
    fn normalize_accepts_http_https_trims_and_strips_slash() {
        assert_eq!(
            normalize_remote_server_url("  https://example.com/  ").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            normalize_remote_server_url("HTTP://LOCALHOST:8080").unwrap(),
            "HTTP://LOCALHOST:8080"
        );
    }

    #[test]
    fn normalize_rejects_bad_urls() {
        assert!(normalize_remote_server_url("").is_err());
        assert!(normalize_remote_server_url("   ").is_err());
        assert!(normalize_remote_server_url("ftp://x").is_err());
        assert!(normalize_remote_server_url("https://").is_err());
        assert!(normalize_remote_server_url("https:///path").is_err());
        assert!(normalize_remote_server_url("not-a-url").is_err());
        assert!(normalize_remote_server_url("https://ex ample.com").is_err());
        assert!(normalize_remote_server_url("https://example.com/foo\nbar").is_err());
    }
}
