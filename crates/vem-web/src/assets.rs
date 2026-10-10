//! Embedded frontend assets with SPA fallback. Implemented by Task 6.

use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};

pub async fn static_handler(_uri: Uri) -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}
