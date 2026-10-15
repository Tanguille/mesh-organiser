use std::env;

fn main() {
    // Set DATABASE_URL for sqlx compile-time query checking
    // This allows sqlx::query! macros to verify queries at compile time
    let db_path = std::path::Path::new("model.sqlite");
    if db_path.exists() {
        // Get absolute path without using canonicalize to avoid Windows path issues
        let current_dir = env::current_dir().expect("Failed to get current directory");
        let absolute_path = current_dir.join("model.sqlite");

        let path = absolute_path.to_str().unwrap().replace('\\', "/");
        let db_url = format!("sqlite:///{path}");
        println!("cargo:rustc-env=DATABASE_URL={db_url}");
        println!("cargo:rerun-if-changed=model.sqlite");
    } else {
        println!(
            "cargo:warning=Database file not found at model.sqlite, sqlx compile-time checking may fail"
        );
        println!(
            "cargo:warning=Run migrations or create the database file to enable compile-time query checking"
        );
    }
}
