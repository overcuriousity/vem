mod common;
use axum::body::Body;
use axum::http::{Method, StatusCode};
use common::*;

async fn get(app: &axum::Router, path: &str) -> (StatusCode, String, String) {
    let (s, h, b) = send(app, req(Method::GET, path).body(Body::empty()).unwrap()).await;
    (
        s,
        h.get("content-type")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default(),
        String::from_utf8_lossy(&b).to_string(),
    )
}

#[tokio::test]
async fn spa_fallback_and_api_404() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    for path in ["/", "/sessions/3", "/activity/files?obs=2"] {
        let (s, ct, body) = get(&app, path).await;
        assert_eq!(s, StatusCode::OK, "{path}");
        assert!(ct.starts_with("text/html"), "{path}: {ct}");
        assert!(
            body.contains("<html") || body.contains("<!doctype html"),
            "{path}"
        );
    }
    let (s, ct, body) = get(&app, "/api/nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(ct.starts_with("application/json"));
    assert!(body.contains("not_found"));
}

#[tokio::test]
async fn no_path_escapes_the_embedded_folder() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    for path in [
        "/assets/../../Cargo.toml",
        "/..%2f..%2fCargo.toml",
        "/%2e%2e/%2e%2e/Cargo.toml",
        "/assets/..\\..\\Cargo.toml",
    ] {
        let (_, _, body) = get(&app, path).await;
        assert!(
            !body.contains("[workspace]"),
            "{path} leaked a file outside frontend/dist"
        );
    }
}
