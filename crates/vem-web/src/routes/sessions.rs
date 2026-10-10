//! Session list, detail, messages, tool calls and observations. Implemented by Task 7.

use crate::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}
