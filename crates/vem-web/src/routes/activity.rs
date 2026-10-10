//! Activity views (commands, file operations, indicators), retained blobs and blob diffs.

use crate::{ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use vem_case::{diff, query, CaseError};

const COMMANDS: &[&str] = &["command_executed"];
const FILES: &[&str] = &["file_read", "file_written", "file_edited", "file_deleted"];
const INDICATORS: &[&str] = &[
    "url_referenced",
    "secret_candidate",
    "paste_detected",
    "upload_detected",
];
const BLOB_CAP: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
struct ActivityParams {
    root: Option<i64>,
    session: Option<i64>,
    kind: Option<String>,
}

#[derive(Serialize)]
struct BlobContent {
    sha256: String,
    size: u64,
    binary: bool,
    truncated: bool,
    text: Option<String>,
    bytes_b64: Option<String>,
}

#[derive(Deserialize)]
struct DiffParams {
    before: Option<String>,
    after: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/activity/{tab}", get(activity))
        .route("/api/blobs/{sha}", get(blob))
        .route("/api/diff", get(diff_blobs))
}

async fn activity(
    State(s): State<AppState>,
    Path(tab): Path<String>,
    Query(p): Query<ActivityParams>,
) -> Result<Json<Vec<query::ActivityRow>>, ApiError> {
    let kinds: Vec<String> = match (tab.as_str(), p.kind.as_deref()) {
        ("commands", _) => COMMANDS.iter().map(|k| k.to_string()).collect(),
        ("files", _) => FILES.iter().map(|k| k.to_string()).collect(),
        ("indicators", None) => INDICATORS.iter().map(|k| k.to_string()).collect(),
        ("indicators", Some(k)) if INDICATORS.contains(&k) => vec![k.to_string()],
        ("indicators", Some(k)) => {
            return Err(ApiError::unprocessable(format!(
                "{k:?} is not an indicator kind"
            )))
        }
        _ => return Err(ApiError::not_found(format!("no activity view {tab:?}"))),
    };
    let f = query::ObservationFilter {
        root_id: p.root,
        session_id: p.session,
        kind: None,
        kinds,
    };
    s.with_case(move |c| query::activity(c, &f)).await.map(Json)
}

/// The text up to the last complete UTF-8 character (a capped read may cut one).
fn text_prefix(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(t) => t.to_string(),
        Err(e) => String::from_utf8_lossy(&bytes[..e.valid_up_to()]).to_string(),
    }
}

async fn blob(
    State(s): State<AppState>,
    Path(sha): Path<String>,
) -> Result<Json<BlobContent>, ApiError> {
    let b = s
        .with_own_case(move |c| {
            query::blob_bytes(c, &sha, BLOB_CAP)?
                .ok_or_else(|| CaseError::NotFound(format!("blob {sha}")))
        })
        .await?;
    let binary = diff::is_binary(&b.bytes);
    Ok(Json(BlobContent {
        sha256: b.sha256,
        size: b.size,
        binary,
        truncated: b.truncated,
        text: (!binary).then(|| text_prefix(&b.bytes)),
        bytes_b64: binary.then(|| base64::engine::general_purpose::STANDARD.encode(&b.bytes)),
    }))
}

async fn diff_blobs(
    State(s): State<AppState>,
    Query(p): Query<DiffParams>,
) -> Result<Json<diff::DiffResult>, ApiError> {
    if p.before.is_none() && p.after.is_none() {
        return Err(ApiError::unprocessable("give `before`, `after` or both"));
    }
    s.with_own_case(move |c| {
        let side = |sha: &Option<String>| -> Result<Vec<u8>, CaseError> {
            let Some(sha) = sha else {
                return Ok(Vec::new());
            };
            let b = query::blob_bytes(c, sha, diff::MAX_DIFF_BYTES)?
                .ok_or_else(|| CaseError::NotFound(format!("blob {sha}")))?;
            if b.truncated {
                return Err(CaseError::Invalid(format!(
                    "blob {sha} is larger than {} bytes and is not diffed",
                    diff::MAX_DIFF_BYTES
                )));
            }
            Ok(b.bytes)
        };
        let (old, new) = (side(&p.before)?, side(&p.after)?);
        Ok(diff::diff_bytes(&old, &new))
    })
    .await
    .map(Json)
}
