//! The detail drawer: one message with its tool calls, observations and annotations; provenance rows; and
//! verified raw bytes at the provenance offset.

use crate::{ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use vem_case::{annotations, query, CaseError};

#[derive(Serialize)]
struct MessageDetail {
    message: query::MessageRow,
    tool_calls: Vec<query::ToolCallRow>,
    observations: Vec<query::ObservationRow>,
    annotations: Vec<annotations::Annotation>,
}

#[derive(Serialize)]
struct RawWindow {
    total_length: u64,
    offset: u64,
    bytes_b64: String,
    /// Always true in a 200 response: the whole record was re-hashed against its provenance hash first.
    verified: bool,
}

#[derive(Deserialize)]
struct RawParams {
    offset: Option<u64>,
    len: Option<u64>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/messages/{id}", get(message))
        .route("/api/provenance/{id}", get(provenance))
        .route("/api/provenance/{id}/raw", get(raw))
}

async fn message(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<MessageDetail>, ApiError> {
    s.with_case(move |c| {
        let message =
            query::message(c, id)?.ok_or_else(|| CaseError::NotFound(format!("message {id}")))?;
        Ok(MessageDetail {
            tool_calls: query::tool_calls_for_message(c, id)?,
            observations: query::observations_for_message(c, id)?,
            annotations: annotations::list_for_message(c, id)?,
            message,
        })
    })
    .await
    .map(Json)
}

async fn provenance(
    State(s): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<query::ProvenanceRow>, ApiError> {
    s.with_case(move |c| {
        query::provenance(c, id)?.ok_or_else(|| CaseError::NotFound(format!("provenance {id}")))
    })
    .await
    .map(Json)
}

async fn raw(
    State(s): State<AppState>,
    Path(id): Path<i64>,
    Query(p): Query<RawParams>,
) -> Result<Json<RawWindow>, ApiError> {
    let w = s
        .with_own_case(move |c| {
            query::raw_window(
                c,
                id,
                p.offset.unwrap_or(0),
                p.len.unwrap_or(query::RAW_WINDOW_MAX),
            )
        })
        .await?;
    Ok(Json(RawWindow {
        total_length: w.total_length,
        offset: w.offset,
        bytes_b64: base64::engine::general_purpose::STANDARD.encode(&w.bytes),
        verified: true,
    }))
}
