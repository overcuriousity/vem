//! Examiner annotations: list, create, delete. Every write is audited by `vem_case::annotations`.

use crate::{ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::Deserialize;
use vem_case::annotations::{self, Annotation};

#[derive(Deserialize)]
struct ListParams {
    target_type: Option<String>,
    target_id: Option<i64>,
}

#[derive(Deserialize)]
struct NewAnnotation {
    target_type: String,
    target_id: i64,
    kind: String,
    value: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/annotations", get(list).post(create))
        .route("/api/annotations/{id}", delete(remove))
}

async fn list(
    State(s): State<AppState>,
    Query(p): Query<ListParams>,
) -> Result<Json<Vec<Annotation>>, ApiError> {
    s.with_case(move |c| annotations::list(c, p.target_type.as_deref(), p.target_id))
        .await
        .map(Json)
}

async fn create(
    State(s): State<AppState>,
    Json(a): Json<NewAnnotation>,
) -> Result<(StatusCode, Json<Annotation>), ApiError> {
    let created = s
        .with_case(move |c| annotations::create(c, &a.target_type, a.target_id, &a.kind, &a.value))
        .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn remove(State(s): State<AppState>, Path(id): Path<i64>) -> Result<StatusCode, ApiError> {
    s.with_case(move |c| annotations::delete(c, id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
