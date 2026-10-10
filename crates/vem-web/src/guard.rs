//! Loopback guards: Host and Origin checks, security headers. Implemented by Task 6.

use crate::AppState;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;

pub async fn guard(State(_state): State<AppState>, req: Request, next: Next) -> Response {
    next.run(req).await
}
