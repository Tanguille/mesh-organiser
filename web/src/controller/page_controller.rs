use axum::{
    Router,
    extract::{Path, State},
    response::Html,
    routing::get,
};
use tokio::fs;
use tower_http::services::ServeFile;

use service::AppState;

use crate::{controller::share_controller::resolve_share_owner, error::ApplicationError};

/// Serves the SPA shell for client-side routes; unauthenticated, the SPA handles login itself.
pub fn router() -> Router<AppState> {
    let index = ServeFile::new("www/index.html");
    let sub_index = ServeFile::new("www/group/1.html");

    Router::new()
        .route_service("/about", index.clone())
        .route_service("/settings", index.clone())
        .route_service("/favorite", index.clone())
        .route_service("/group", index.clone())
        .route_service("/import", index.clone())
        .route_service("/login", index.clone())
        .route_service("/model", index.clone())
        .route_service("/printed", index.clone())
        .route_service("/resource", index)
        .route_service("/group/", sub_index.clone())
        .route_service("/label/", sub_index.clone())
        .route_service("/share/", sub_index.clone())
        .route_service("/group/{group_id}", sub_index.clone())
        .route_service("/label/{label_id}", sub_index)
        .route("/share/{share_id}", get(serve_share_page))
}

async fn serve_share_page(
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Html<String>, ApplicationError> {
    let mut html = fs::read_to_string("www/group/1.html").await?;

    let Ok((share, user)) = resolve_share_owner(&app_state, &share_id).await else {
        return Ok(Html(html));
    };

    html = html
        .replace(
            "content=\"Mesh Organiser\"",
            &format!(
                "content=\"Share: {}\"",
                escape_html_attribute(&share.share_name)
            ),
        )
        .replace(
            "content=\"A personal 3d printing model library.\"",
            &format!(
                "content=\"Shared by user {}. Contains {} model{}.\"",
                escape_html_attribute(&user.username),
                share.model_ids.len(),
                if share.model_ids.len() >= 2 { "s" } else { "" }
            ),
        );

    Ok(Html(html))
}

/// Share and user names are user-controlled and get spliced into `<meta content="...">`
/// attributes, so every character that could break out of the attribute or the tag is escaped.
fn escape_html_attribute(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for char in text.chars() {
        match char {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(char),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    #[test]
    fn escape_html_attribute_escapes_special_chars() {
        assert_eq!(
            super::escape_html_attribute(r#"x" <b>&'"#),
            "x&quot; &lt;b&gt;&amp;&#39;"
        );
    }

    #[test]
    fn escape_html_attribute_leaves_alphanumerics_unchanged() {
        assert_eq!(super::escape_html_attribute("Share42"), "Share42");
    }
}
