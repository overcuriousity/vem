//! Export run, list and download. Implemented by Task 8.

use crate::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}
