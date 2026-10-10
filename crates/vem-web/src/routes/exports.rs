//! Exports: run one into `<case>/exports/` (hashed, audited), list earlier ones, download by name.

use crate::{ApiError, AppState};
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use vem_case::export::{self, ExportEntry, ExportReport, Format, Scope};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ScopeRequest {
    Case,
    Root { id: i64 },
    Session { id: i64 },
}

#[derive(Deserialize)]
struct ExportRequest {
    format: String,
    scope: ScopeRequest,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/exports", post(run).get(list))
        .route("/api/exports/{name}", get(download))
}

async fn run(
    State(s): State<AppState>,
    Json(r): Json<ExportRequest>,
) -> Result<Json<ExportReport>, ApiError> {
    let format = Format::parse(&r.format)
        .ok_or_else(|| ApiError::unprocessable(format!("unknown export format {:?}", r.format)))?;
    let scope = match r.scope {
        ScopeRequest::Case => Scope::Case,
        ScopeRequest::Root { id } => Scope::Root(id),
        ScopeRequest::Session { id } => Scope::Session(id),
    };
    s.with_own_case(move |c| {
        let out = export::default_output(c, format, &scope);
        export::run(c, format, &scope, &out)
    })
    .await
    .map(Json)
}

async fn list(State(s): State<AppState>) -> Result<Json<Vec<ExportEntry>>, ApiError> {
    s.with_case(|c| export::list(c)).await.map(Json)
}

/// The whole file, as an attachment. `export_file` only accepts a plain name present in `<case>/exports/`.
async fn download(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let (bytes, name) = s
        .with_own_case(move |c| {
            let path = export::export_file(c, &name)?;
            Ok((std::fs::read(path)?, name))
        })
        .await?;
    let mime = match name.rsplit('.').next() {
        Some("jsonl") => "application/x-ndjson",
        Some("csv") => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    };
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
        ],
        bytes,
    )
        .into_response())
}
