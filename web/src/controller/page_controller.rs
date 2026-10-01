use axum::{
    Router,
    extract::{Path, State},
    response::Html,
    routing::get,
};
use tokio::fs;
use tower_http::services::ServeFile;

use crate::{
    controller::share_controller::resolve_share_owner, error::ApplicationError,
    web_app_state::WebAppState,
};

pub fn router() -> Router<WebAppState> {
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
    State(app_state): State<WebAppState>,
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
    const DANGEROUS: [char; 5] = ['"', '<', '>', '&', '\''];

    /// Raw `&` can legitimately appear as the start of an entity, so for it we only check that
    /// every `&` is followed by an entity body ending in `;`. The others must be gone entirely.
    fn assert_attribute_safe(escaped: &str) {
        for char in ['"', '<', '>', '\''] {
            assert!(!escaped.contains(char), "raw {char:?} left in {escaped:?}");
        }

        for (position, _) in escaped.match_indices('&') {
            assert!(
                escaped[position..].contains(';'),
                "bare '&' (not an entity) in {escaped:?}"
            );
        }
    }

    #[test]
    fn escape_html_attribute_neutralises_attribute_injection_payload() {
        let escaped = super::escape_html_attribute(r#"x" onload="alert(1)"#);

        assert_attribute_safe(&escaped);
    }

    #[test]
    fn escape_html_attribute_neutralises_script_tag() {
        let escaped = super::escape_html_attribute("<script>alert('x')</script>");

        assert_attribute_safe(&escaped);
        assert!(!escaped.contains("<script"));
    }

    #[test]
    fn escape_html_attribute_escapes_every_dangerous_char() {
        for char in DANGEROUS {
            let escaped = super::escape_html_attribute(&format!("a{char}b"));

            assert_ne!(escaped, format!("a{char}b"), "{char:?} was not escaped");
            assert!(escaped.starts_with('a') && escaped.ends_with('b'));
            if char != '&' {
                assert!(!escaped.contains(char), "raw {char:?} left in {escaped:?}");
            }
        }
    }

    #[test]
    fn escape_html_attribute_leaves_alphanumerics_unchanged() {
        assert_eq!(super::escape_html_attribute("Share42"), "Share42");
    }
}
