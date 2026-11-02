use std::env;

use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::app::App;

mod app;
mod controller;
mod error;
mod mcp;
mod path_safety;
mod query_bounds;
mod session_store;
#[cfg(test)]
mod test_support;
mod user;
mod web_import_state;

const ENV_RUST_LOG: &str = "RUST_LOG";

#[allow(clippy::future_not_send)] // App and its state are not Send; run on main thread via block_on
async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(EnvFilter::new(env::var(ENV_RUST_LOG).unwrap_or_else(
            |_| "axum_login=debug,tower_sessions=debug,sqlx=warn,tower_http=debug".into(),
        )))
        .with(tracing_subscriber::fmt::layer())
        .try_init()?;

    // Fail fast on a bad PORT before touching the database.
    let port = app::parse_port()?;

    App::new().await?.serve(port).await
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(32 * 1024 * 1024)
        .build()
        .expect("failed to build tokio runtime");

    if let Err(e) = rt.block_on(async_main()) {
        eprintln!("Fatal: {e}");
        std::process::exit(1);
    }
}
