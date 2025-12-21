fn main() {
    // Set DATABASE_URL for sqlx compile-time query checking
    // This allows sqlx::query! macros to verify queries at compile time
    let db_path = std::path::Path::new("model.sqlite");
    if db_path.exists() {
        // Get absolute path without using canonicalize to avoid Windows path issues
        let current_dir = std::env::current_dir().expect("Failed to get current directory");
        let absolute_path = current_dir.join("model.sqlite");

        // Convert to string and normalize for SQLite URL format
        let path_str = absolute_path.to_str().unwrap().replace('\\', "/");
        // For Windows absolute paths, ensure format is D:/path (no leading slash before drive)
        let normalized_path = if cfg!(windows) && path_str.len() > 2 {
            // On Windows, if we have something like /D:/path, remove the leading slash
            if let Some(stripped) = path_str.strip_prefix('/') {
                let chars: Vec<char> = stripped.chars().collect();
                if chars.len() > 1 && chars[1] == ':' {
                    // Remove leading slash: /D:/path -> D:/path
                    stripped.to_string()
                } else {
                    path_str
                }
            } else {
                path_str
            }
        } else {
            path_str
        };

        let db_url = format!("sqlite:///{}", normalized_path);
        println!("cargo:rustc-env=DATABASE_URL={db_url}");
        println!("cargo:rerun-if-changed=model.sqlite");
    } else {
        // If database doesn't exist, sqlx compile-time checking will fail
        println!(
            "cargo:warning=Database file not found at model.sqlite, sqlx compile-time checking may fail"
        );
        println!(
            "cargo:warning=Run migrations or create the database file to enable compile-time query checking"
        );
    }
}
