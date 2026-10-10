#![allow(dead_code)]
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::Case;

pub const PORT: u16 = 8787;
pub const HOST: &str = "127.0.0.1:8787";
pub const ORIGIN: &str = "http://127.0.0.1:8787";

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code")
        .join(name)
        .canonicalize()
        .unwrap()
}

/// A case in a temp dir with each named fixture attached (retained) and ingested.
pub fn ingested(fixtures: &[&str]) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("case");
    let mut case = Case::create(&dir, "web test", Some("examiner")).unwrap();
    for f in fixtures {
        attach(
            &mut case,
            &fixture(f),
            AttachOptions {
                label: f.to_string(),
                host: None,
                user: None,
                os: None,
                harness: None,
                retain: true,
            },
        )
        .unwrap();
    }
    vem_case::ingest::ingest(&mut case, None).unwrap();
    (tmp, dir)
}

pub fn app(case_dir: &Path) -> Router {
    vem_web::app(case_dir, PORT).unwrap()
}

pub async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, body)
}

pub fn req(method: Method, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", HOST)
}

pub async fn get_json(app: &Router, path: &str) -> (StatusCode, Value) {
    let (s, _, b) = send(app, req(Method::GET, path).body(Body::empty()).unwrap()).await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

pub async fn post_json(app: &Router, path: &str, body: Value) -> (StatusCode, Value) {
    let r = req(Method::POST, path)
        .header("content-type", "application/json")
        .header("origin", ORIGIN)
        .body(Body::from(body.to_string()))
        .unwrap();
    let (s, _, b) = send(app, r).await;
    (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

pub async fn delete(app: &Router, path: &str) -> StatusCode {
    let r = req(Method::DELETE, path)
        .header("content-type", "application/json")
        .header("origin", ORIGIN)
        .body(Body::empty())
        .unwrap();
    send(app, r).await.0
}

pub fn audit_count(case_dir: &Path) -> i64 {
    Case::open(case_dir)
        .unwrap()
        .conn
        .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
        .unwrap()
}
