pub mod auth_controller;
pub mod blob_controller;
pub mod group_controller;
pub mod label_controller;
pub mod model_controller;
pub mod page_controller;
pub mod resource_controller;
pub mod share_controller;
pub mod threemf_controller;
pub mod user_controller;

/// Request body shared by every endpoint that takes just a list of model ids.
#[derive(serde::Deserialize)]
pub struct ModelIdsParams {
    pub model_ids: Vec<i64>,
}

/// Limits `router` per peer ip, for routes anyone can call before authenticating: a burst of 10,
/// then one request every 5 seconds. Each call builds its own limiter, so traffic on one router
/// does not use up another's allowance. The peer ip comes from the connect info `app.rs` serves.
///
/// Spawns the limiter's cleanup task, so it must run inside a tokio runtime.
pub fn rate_limited(router: axum::Router<service::AppState>) -> axum::Router<service::AppState> {
    let config = tower_governor::governor::GovernorConfigBuilder::default()
        .per_second(5)
        .burst_size(10)
        .finish()
        .expect("a nonzero period and burst size always build a config");
    // The limiter keeps an entry for every peer ip it has seen and never drops one by itself;
    // `retain_recent` drops the ones whose allowance has fully refilled.
    let limiter = config.limiter().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_mins(1));
        loop {
            interval.tick().await;
            limiter.retain_recent();
        }
    });

    router.layer(tower_governor::GovernorLayer::new(config))
}
