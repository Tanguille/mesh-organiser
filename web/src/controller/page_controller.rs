use axum::{
    Router,
    extract::{Path, State},
    response::Html,
    routing::get,
};
use tokio::fs;

use service::AppState;

use crate::{controller::share_controller::ShareScope, error::ApplicationError};

/// Serves share links with the share's name in the link-preview meta tags. Every other client-side
/// route gets the plain SPA shell from the static-file fallback in `app.rs`.
pub fn router() -> Router<AppState> {
    Router::new().route("/share/{share_id}", get(serve_share_page))
}

async fn serve_share_page(
    Path(share_id): Path<String>,
    State(app_state): State<AppState>,
) -> Result<Html<String>, ApplicationError> {
    let mut html = fs::read_to_string("www/index.html").await?;

    let Ok(ShareScope { share, owner }) = ShareScope::resolve(&app_state, &share_id).await else {
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
                escape_html_attribute(&owner.username),
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
