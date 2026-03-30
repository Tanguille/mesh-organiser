//! Single place where all HTTP API [`Router`]s are merged.
//!
//! When adding a new controller, register its `router()` here so the surface area stays visible.

use std::{io, io::ErrorKind, sync::Arc};

use axum::Router;
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};

use crate::web_app_state::WebAppState;

use super::{
    auth_controller, blob_controller, group_controller, label_controller, model_controller,
    page_controller, resource_controller, share_controller, slicer_controller, threemf_controller,
    user_controller,
};

/// Builds the combined `/api/v1/...` (and related) API router: auth (rate-limited) plus all resource routers.
pub fn merged_api_router() -> Result<Router<WebAppState>, io::Error> {
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(5)
            .burst_size(10)
            .finish()
            .ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidData, "Failed to create governor config")
            })?,
    );

    let auth_router = auth_controller::router().layer(GovernorLayer::new(governor_config));

    Ok(Router::new()
        .merge(auth_router)
        .merge(blob_controller::router())
        .merge(model_controller::router())
        .merge(group_controller::router())
        .merge(label_controller::router())
        .merge(resource_controller::router())
        .merge(user_controller::router())
        .merge(threemf_controller::router())
        .merge(page_controller::router())
        .merge(share_controller::router())
        .merge(slicer_controller::router()))
}
