mod common;
use axum::body::Body;
use axum::http::Method;
use common::*;
use serde_json::json;

#[tokio::test]
async fn export_run_list_download() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    let audits = audit_count(&dir);
    let (s, r) = post_json(
        &app,
        "/api/exports",
        json!({ "format": "timesketch-jsonl", "scope": { "kind": "case" } }),
    )
    .await;
    assert_eq!(s, 200);
    let name = r["name"].as_str().unwrap().to_string();
    assert!(name.ends_with("-case.jsonl"));
    let expected = vem_case::export::events(
        &vem_case::Case::open(&dir).unwrap(),
        &vem_case::export::Scope::Case,
    )
    .unwrap()
    .len();
    assert_eq!(r["events"], expected);
    assert_eq!(audit_count(&dir), audits + 1);

    let (_, list) = get_json(&app, "/api/exports").await;
    assert_eq!(
        (list[0]["name"].as_str(), list[0]["exists"].as_bool()),
        (Some(name.as_str()), Some(true))
    );

    let (s, h, body) = send(
        &app,
        req(Method::GET, &format!("/api/exports/{name}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, 200);
    assert!(h
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .contains(&name));
    assert_eq!(
        vem_core::hash::sha256_hex(&body),
        r["sha256"].as_str().unwrap()
    );
    assert_eq!(String::from_utf8(body).unwrap().lines().count(), expected);

    for bad in ["..%2Fcase.db", "nope.jsonl", "%2E%2E"] {
        assert_eq!(
            get_json(&app, &format!("/api/exports/{bad}")).await.0,
            404,
            "{bad}"
        );
    }
    let (s, _) = post_json(
        &app,
        "/api/exports",
        json!({ "format": "xml", "scope": { "kind": "case" } }),
    )
    .await;
    assert_eq!(s, 422);
    let (s, _) = post_json(
        &app,
        "/api/exports",
        json!({ "format": "timesketch-csv", "scope": { "kind": "session", "id": 999999 } }),
    )
    .await;
    assert_eq!(s, 404);
    let (s, r2) = post_json(
        &app,
        "/api/exports",
        json!({ "format": "vestigo-parquet", "scope": { "kind": "root", "id": 1 } }),
    )
    .await;
    assert_eq!(s, 200);
    assert!(r2["name"].as_str().unwrap().ends_with("-root-1.parquet"));
}
