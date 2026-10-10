//! API errors: `{ "error": message, "kind": kind }` with a status code.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use vem_case::CaseError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub kind: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind,
            message: message.into(),
        }
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
    pub fn unprocessable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", message)
    }
    pub(crate) fn join(e: tokio::task::JoinError) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            format!("worker failed: {e}"),
        )
    }
}

impl From<CaseError> for ApiError {
    fn from(e: CaseError) -> Self {
        let message = e.to_string();
        match e {
            CaseError::NotFound(_) | CaseError::NoSuchRoot(_) | CaseError::NoSuchSession(_) => {
                Self::not_found(message)
            }
            CaseError::IntegrityMismatch(_) => {
                Self::new(StatusCode::CONFLICT, "integrity_mismatch", message)
            }
            CaseError::Invalid(_) | CaseError::Export(_) | CaseError::InsideEvidence { .. } => {
                Self::unprocessable(message)
            }
            _ => Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message, "kind": self.kind })),
        )
            .into_response()
    }
}

/// A handler panic becomes a 500 with a JSON body instead of a dropped connection.
pub fn panic_response(_: Box<dyn std::any::Any + Send + 'static>) -> Response {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "panic",
        "internal error: a handler panicked",
    )
    .into_response()
}
