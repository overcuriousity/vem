//! Message detail, provenance and raw bytes. Implemented by Task 7.

use crate::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}
