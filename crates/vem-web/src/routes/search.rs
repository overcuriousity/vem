//! Case-wide full-text search over message text and tool calls.

use crate::{ApiError, AppState};
use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use vem_case::query;

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
    limit: Option<usize>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/search", get(search))
}

async fn search(
    State(s): State<AppState>,
    Query(p): Query<SearchParams>,
) -> Result<Json<Vec<query::SearchHit>>, ApiError> {
    let q = p.q.unwrap_or_default();
    let limit = p.limit.unwrap_or(200).clamp(1, 1000);
    s.with_case(move |c| query::search(c, &q, limit))
        .await
        .map(Json)
}
