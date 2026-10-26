fn main() {
    // Cargo cannot otherwise see the SQLite schema used by sqlx::query! at compile time.
    println!("cargo:rerun-if-changed=model.sqlite");
}
