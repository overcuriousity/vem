//! Loopback guards. The server binds 127.0.0.1, but a browser can still be steered at it: DNS rebinding
//! (a foreign name resolving to 127.0.0.1) is stopped by the Host check, cross-site writes from another
//! page by the Content-Type and Origin checks. Every response gets the security headers.

use crate::{ApiError, AppState};
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

fn add_headers(res: &mut Response, api: bool) {
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if api {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

pub async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let port = state.port();
    let api = req.uri().path().starts_with("/api/") || req.uri().path() == "/api";
    // Scoped so the borrow of `req` (whose body is not `Sync`) ends before the await below.
    let rejection = {
        let header_str = |name: header::HeaderName| {
            req.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let host = header_str(header::HOST).unwrap_or_default();
        let local_hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        if !local_hosts.contains(&host) {
            Some(ApiError::new(
                StatusCode::MISDIRECTED_REQUEST,
                "bad_host",
                format!(
                    "refusing Host {host:?}: vem answers only to 127.0.0.1:{port} or localhost:{port}"
                ),
            ))
        } else if req.method() != Method::GET && req.method() != Method::HEAD {
            let json = header_str(header::CONTENT_TYPE)
                .is_some_and(|c| c.to_ascii_lowercase().starts_with("application/json"));
            let origin_ok = header_str(header::ORIGIN)
                .is_none_or(|o| local_hosts.iter().any(|h| o == format!("http://{h}")));
            if !json {
                Some(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "writes must be sent as application/json",
                ))
            } else if !origin_ok {
                Some(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "cross-origin writes are refused",
                ))
            } else {
                None
            }
        } else {
            None
        }
    };
    let mut res = match rejection {
        Some(e) => e.into_response(),
        None => next.run(req).await,
    };
    add_headers(&mut res, api);
    res
}
