//! Case overview, evidence roots with stores and ingest status, the file inventory, anomalies, audit log.

use crate::{ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use vem_case::{query, CaseError};

#[derive(Serialize)]
struct CaseOverview {
    info: vem_case::CaseInfo,
    totals: query::CaseTotals,
}

#[derive(Deserialize)]
struct AnomalyParams {
    root: Option<i64>,
    kind: Option<String>,
    severity: Option<String>,
    session: Option<i64>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/case", get(case_overview))
        .route("/api/roots", get(roots))
        .route("/api/roots/{id}/files", get(root_files))
        .route("/api/anomalies", get(anomalies))
        .route("/api/audit", get(audit))
}

async fn case_overview(State(s): State<AppState>) -> Result<Json<CaseOverview>, ApiError> {
    s.with_case(|c| {
        Ok(CaseOverview {
            info: c.info()?,
            totals: query::totals(c)?,
        })
    })
    .await
    .map(Json)
}

async fn roots(State(s): State<AppState>) -> Result<Json<Vec<query::RootOverview>>, ApiError> {
    s.with_case(|c| query::root_overviews(c)).await.map(Json)
}

async fn root_files(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Vec<query::SourceFileRow>>, ApiError> {
    s.with_case(move |c| {
        if !query::roots(c)?.iter().any(|r| r.id == id) {
            return Err(CaseError::NoSuchRoot(id));
        }
        query::source_files(c, id)
    })
    .await
    .map(Json)
}

async fn anomalies(
    State(s): State<AppState>,
    Query(p): Query<AnomalyParams>,
) -> Result<Json<Vec<query::AnomalyRow>>, ApiError> {
    s.with_case(move |c| {
        query::anomalies(
            c,
            &query::AnomalyFilter {
                root_id: p.root,
                kind: p.kind,
                severity: p.severity,
                session_id: p.session,
            },
        )
    })
    .await
    .map(Json)
}

async fn audit(State(s): State<AppState>) -> Result<Json<Vec<query::AuditRow>>, ApiError> {
    s.with_case(|c| query::audit_log(c)).await.map(Json)
}
