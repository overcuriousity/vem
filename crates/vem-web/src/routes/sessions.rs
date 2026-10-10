//! Session list with filters, session detail (ancestors, children, identity claims), and per-session
//! messages, tool calls and observations.

use crate::{ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use vem_case::{query, Case, CaseError};

#[derive(Deserialize)]
struct SessionParams {
    harness: Option<String>,
    root: Option<i64>,
    kind: Option<String>,
    project: Option<String>,
    from: Option<String>,
    to: Option<String>,
    has_children: Option<bool>,
    has_anomalies: Option<bool>,
}

#[derive(Deserialize)]
struct MetaParam {
    meta: Option<bool>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/sessions", get(list))
        .route("/api/sessions/{id}", get(detail))
        .route("/api/sessions/{id}/messages", get(messages))
        .route("/api/sessions/{id}/tool-calls", get(tool_calls))
        .route("/api/sessions/{id}/observations", get(observations))
}

fn require_session(c: &Case, id: i64) -> Result<(), CaseError> {
    query::session(c, id)?
        .map(|_| ())
        .ok_or(CaseError::NoSuchSession(id))
}

async fn list(
    State(s): State<AppState>,
    Query(p): Query<SessionParams>,
) -> Result<Json<Vec<query::SessionRow>>, ApiError> {
    let f = query::SessionFilter {
        root_id: p.root,
        harness: p.harness,
        kind: p.kind,
        project_contains: p.project.filter(|x| !x.is_empty()),
        from: p.from.filter(|x| !x.is_empty()),
        to: p.to.filter(|x| !x.is_empty()),
        has_children: p.has_children.unwrap_or(false),
        has_anomalies: p.has_anomalies.unwrap_or(false),
    };
    s.with_case(move |c| query::sessions(c, &f)).await.map(Json)
}

async fn detail(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<query::SessionDetail>, ApiError> {
    s.with_case(move |c| query::session_detail(c, id)?.ok_or(CaseError::NoSuchSession(id)))
        .await
        .map(Json)
}

async fn messages(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Query(p): Query<MetaParam>,
) -> Result<Json<Vec<query::MessageRow>>, ApiError> {
    s.with_case(move |c| {
        require_session(c, id)?;
        query::messages(c, id, p.meta.unwrap_or(false))
    })
    .await
    .map(Json)
}

async fn tool_calls(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<query::ToolCallRow>>, ApiError> {
    s.with_case(move |c| {
        require_session(c, id)?;
        query::tool_calls(c, id)
    })
    .await
    .map(Json)
}

async fn observations(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<query::ObservationRow>>, ApiError> {
    s.with_case(move |c| {
        require_session(c, id)?;
        query::observations(
            c,
            &query::ObservationFilter {
                session_id: Some(id),
                ..Default::default()
            },
        )
    })
    .await
    .map(Json)
}
