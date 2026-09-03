use std::{env, error::Error, process, time::Duration};

use tokio::time;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use service::export_service;

use crate::app::App;

mod app;
mod controller;
mod error;
mod path_safety;
mod query_bounds;
mod user;
mod web_app_state;
mod web_import_state;

const ENV_RUST_LOG: &str = "RUST_LOG";

async fn loop_remove_temp_paths() {
    loop {
        time::sleep(Duration::from_hours(1)).await;
        let _ = export_service::remove_stale_temp_dirs();
    }
}

#[allow(clippy::future_not_send)] // App and its state are not Send; run on main thread via block_on
async fn async_main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::registry()
        .with(EnvFilter::new(env::var(ENV_RUST_LOG).unwrap_or_else(
            |_| "axum_login=debug,tower_sessions=debug,sqlx=warn,tower_http=debug".into(),
        )))
        .with(tracing_subscriber::fmt::layer())
        .try_init()?;

    tokio::spawn(loop_remove_temp_paths());

    App::new().await?.serve().await
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(32 * 1024 * 1024)
        .build()
        .expect("failed to build tokio runtime");

    if let Err(e) = rt.block_on(async_main()) {
        eprintln!("Fatal: {e}");
        process::exit(1);
    }
}
