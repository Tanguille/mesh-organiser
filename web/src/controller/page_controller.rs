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
            &format!("content=\"Share: {}\"", escape_attr(&share.share_name)),
        )
        .replace(
            "content=\"A personal 3d printing model library.\"",
            &format!(
                "content=\"Shared by user {}. Contains {} model{}.\"",
                escape_attr(&user.username),
                share.model_ids.len(),
                if share.model_ids.len() >= 2 { "s" } else { "" }
            ),
        );

    Ok(Html(html))
}

/// Share and user names are user-controlled and get spliced into `<meta content="...">`
/// attributes, so every character that could break out of the attribute or the tag is escaped.
fn escape_attr(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::escape_attr;

    const DANGEROUS: [char; 5] = ['"', '<', '>', '&', '\''];

    /// Raw `&` can legitimately appear as the start of an entity, so for it we only check that
    /// every `&` is followed by an entity body ending in `;`. The others must be gone entirely.
    fn assert_attribute_safe(escaped: &str) {
        for c in ['"', '<', '>', '\''] {
            assert!(!escaped.contains(c), "raw {c:?} left in {escaped:?}");
        }

        for (i, _) in escaped.match_indices('&') {
            assert!(
                escaped[i..].contains(';'),
                "bare '&' (not an entity) in {escaped:?}"
            );
        }
    }

    #[test]
    fn escape_attr_neutralises_attribute_injection_payload() {
        let escaped = escape_attr(r#"x" onload="alert(1)"#);

        assert_attribute_safe(&escaped);
    }

    #[test]
    fn escape_attr_neutralises_script_tag() {
        let escaped = escape_attr("<script>alert('x')</script>");

        assert_attribute_safe(&escaped);
        assert!(!escaped.contains("<script"));
    }

    #[test]
    fn escape_attr_escapes_every_dangerous_char() {
        for c in DANGEROUS {
            let escaped = escape_attr(&format!("a{c}b"));

            assert_ne!(escaped, format!("a{c}b"), "{c:?} was not escaped");
            assert!(escaped.starts_with('a') && escaped.ends_with('b'));
            if c != '&' {
                assert!(!escaped.contains(c), "raw {c:?} left in {escaped:?}");
            }
        }
    }

    #[test]
    fn escape_attr_leaves_alphanumerics_unchanged() {
        assert_eq!(escape_attr("Share42"), "Share42");
    }
}
