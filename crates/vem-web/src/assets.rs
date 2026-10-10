//! The frontend, embedded from `frontend/dist` (read from disk in debug builds). Client-side routes fall
//! back to `index.html`; without a built frontend a short page says how to build it.

use crate::ApiError;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::RustEmbed)]
#[folder = "../../frontend/dist"]
#[allow_missing = true]
struct Dist;

const NOT_BUILT: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>vem</title></head><body>\
<h1>vem</h1><p>The frontend is not built into this binary. Build it with <code>scripts/build-release.sh</code> \
(or <code>cd frontend &amp;&amp; npm ci &amp;&amp; npm run build</code>, then rebuild). The JSON API under <code>/api/</code> is available.</p></body></html>";

fn html(body: impl Into<axum::body::Body>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body.into(),
    )
        .into_response()
}

pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path == "api" || path.starts_with("api/") {
        return ApiError::not_found(format!("no API route {}", uri.path())).into_response();
    }
    // Never hand a path with a parent component, a backslash or an encoded byte to the embed lookup.
    let plain = !path.is_empty()
        && !path.split('/').any(|c| c == ".." || c == ".")
        && !path.contains('\\')
        && !path.contains('%');
    if plain {
        if let Some(file) = Dist::get(path) {
            let mime = file.metadata.mimetype().to_string();
            return ([(header::CONTENT_TYPE, mime)], file.data.into_owned()).into_response();
        }
    }
    match Dist::get("index.html") {
        Some(file) => html(file.data.into_owned()),
        None => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            NOT_BUILT,
        )
            .into_response(),
    }
}
