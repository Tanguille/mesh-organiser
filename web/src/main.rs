use std::{env, fs, time::Duration};

use tokio::time;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{app::App, error::ApplicationError};

mod app;
mod controller;
mod error;
mod path_safety;
mod query_bounds;
mod user;
mod web_app_state;
mod web_import_state;

fn remove_temp_paths() -> Result<(), ApplicationError> {
    let threshold = Duration::from_mins(5);
    let now = std::time::SystemTime::now();
    for entry in fs::read_dir(env::temp_dir())? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir()
            && path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("meshorganiser_")
            && let Ok(metadata) = fs::metadata(&path)
            && let Ok(modified) = metadata.modified()
            && now.duration_since(modified).unwrap_or(Duration::ZERO) >= threshold
        {
            println!("Removing temporary path {}", path.display());
            fs::remove_dir_all(&path)?;
        }
    }

    Ok(())
}

async fn loop_remove_temp_paths() {
    loop {
        time::sleep(Duration::from_hours(1)).await;
        let _ = remove_temp_paths();
    }
}

#[allow(clippy::future_not_send)] // App and its state are not Send; run on main thread via block_on
async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(EnvFilter::new(env::var("RUST_LOG").unwrap_or_else(|_| {
            "axum_login=debug,tower_sessions=debug,sqlx=warn,tower_http=debug".into()
        })))
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
        std::process::exit(1);
    }
}
