//! Activity views, blobs and diffs. Implemented by Task 8.

use crate::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}
