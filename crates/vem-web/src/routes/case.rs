//! Case overview, roots, inventory, anomalies and audit log. Implemented by Task 7.

use crate::AppState;
use axum::Router;

pub fn routes() -> Router<AppState> {
    Router::new()
}
