use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use axum_login::login_required;
use serde::Deserialize;

use db::{
    model::{
        share::{Share, ShareDto},
        user::User,
    },
    share_db, time_now, user_db,
};
use service::AppState;

use crate::{
    error::ApplicationError,
    user::{Backend, CurrentUser},
};

/// A share resolved together with its owning user, plus the model scoping that
/// every share endpoint must apply: a share link only grants the models listed
/// on the share, never everything its owner has.
pub struct ShareScope {
    pub share: Share,
    pub owner: User,
}

impl ShareScope {
    /// Resolves a share and its owning user, erroring if the owner no longer exists.
    pub async fn resolve(app_state: &AppState, share_id: &str) -> Result<Self, ApplicationError> {
        let share = share_db::get_share_via_id(&app_state.db, share_id).await?;

        let Some(owner) = user_db::get_user_by_id(&app_state.db, share.user_id).await? else {
            return Err(ApplicationError::InternalError(
                "Share owner user not found.".into(),
            ));
        };

        Ok(Self { share, owner })
    }

    /// Whether `model_id` is one of the share's models.
    pub fn allows(&self, model_id: i64) -> bool {
        self.share.model_ids.contains(&model_id)
    }

    /// Ids a share request may query: all of the share's ids when nothing
    /// specific was requested, otherwise only the requested ids that belong to
    /// the share, in the share's order. Callers must pass the result on as
    /// `Some(..)` even when empty, never through `none_if_empty`.
    pub fn restrict(&self, requested: &[i64]) -> Vec<i64> {
        if requested.is_empty() {
            return self.share.model_ids.clone();
        }

        let requested: std::collections::HashSet<i64> = requested.iter().copied().collect();

        self.share
            .model_ids
            .iter()
            .copied()
            .filter(|model_id| requested.contains(model_id))
            .collect()
    }
}

/// Routes for the share endpoints; `login_required!` guards only those registered before it.
pub fn router() -> Router<AppState> {
    Router::new().nest(
        "/api/v1",
        Router::new()
            .route("/shares", get(get_shares))
            .route("/shares", post(create_share))
            .route("/shares/{share_id}", put(edit_share))
            .route("/shares/{share_id}", delete(delete_share))
            .route("/shares/{share_id}/models", put(set_model_ids_on_share))
            .route_layer(login_required!(Backend))
            .route("/shares/{share_id}", get(get_share)),
    )
}

pub async fn get_shares(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    let shares = share_db::get_shares(&app_state.db, &user).await?;

    let shares: Vec<ShareDto> = shares
        .into_iter()
        .map(|share| share.to_dto(user.username.clone()))
        .collect();

    Ok(Json(shares).into_response())
}

pub async fn get_share(
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Response, ApplicationError> {
    let scope = ShareScope::resolve(&app_state, &share_id).await?;

    let share = scope.share.to_dto(scope.owner.username);

    Ok(Json(share).into_response())
}

#[derive(Deserialize)]
pub struct CreateShareParams {
    pub share_name: String,
}

pub async fn create_share(
    CurrentUser(user): CurrentUser,
    State(app_state): State<AppState>,
    Json(params): Json<CreateShareParams>,
) -> Result<Response, ApplicationError> {
    let share_id = share_db::create_share(&app_state.db, &user, &params.share_name).await?;

    Ok(Json(ShareDto {
        id: share_id,
        share_name: params.share_name,
        user_name: user.username,
        model_ids: Vec::new(),
        created_at: time_now(),
    })
    .into_response())
}

#[derive(Deserialize)]
pub struct EditShareParams {
    pub share_name: String,
}

pub async fn edit_share(
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
    Json(params): Json<EditShareParams>,
) -> Result<StatusCode, ApplicationError> {
    share_db::rename_share(&app_state.db, &user, &share_id, &params.share_name).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_model_ids_on_share(
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
    Json(params): Json<crate::controller::ModelIdsParams>,
) -> Result<StatusCode, ApplicationError> {
    share_db::set_model_ids_on_share(&app_state.db, &user, &share_id, params.model_ids).await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_share(
    CurrentUser(user): CurrentUser,
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
) -> Result<StatusCode, ApplicationError> {
    share_db::delete_share(&app_state.db, &user, &share_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    fn scope(model_ids: Vec<i64>) -> super::ShareScope {
        super::ShareScope {
            share: db::model::share::Share {
                id: String::new(),
                created_at: String::new(),
                share_name: String::new(),
                user_id: 1,
                model_ids,
            },
            owner: db::model::user::User::default(),
        }
    }

    #[test]
    fn allows_only_members_of_the_share() {
        assert!(scope(vec![1, 2, 3]).allows(2));
        assert!(!scope(vec![1, 2, 3]).allows(4));
        assert!(!scope(vec![]).allows(1));
    }

    #[test]
    fn restrict_empty_request_returns_every_share_id() {
        assert_eq!(scope(vec![1, 2, 3]).restrict(&[]), vec![1, 2, 3]);
        assert_eq!(scope(vec![]).restrict(&[]), Vec::<i64>::new());
    }

    #[test]
    fn restrict_subset_request_returns_only_shared_ids() {
        assert_eq!(scope(vec![1, 2, 3]).restrict(&[2, 3]), vec![2, 3]);
        assert_eq!(scope(vec![1, 2]).restrict(&[2, 999]), vec![2]);
    }

    // The leak scenario: an empty result must stay empty (and be passed on as
    // `Some(vec![])`), because `None` would mean "no restriction" downstream.
    #[test]
    fn restrict_foreign_ids_return_empty() {
        assert_eq!(scope(vec![1, 2]).restrict(&[3, 4]), Vec::<i64>::new());
        assert_eq!(scope(vec![]).restrict(&[1]), Vec::<i64>::new());
    }

    #[test]
    fn restrict_keeps_share_order_and_ignores_requested_duplicates() {
        assert_eq!(scope(vec![1, 2, 3]).restrict(&[3, 1, 3, 1]), vec![1, 3]);
    }
}
