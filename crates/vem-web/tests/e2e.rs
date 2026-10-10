mod common;
use axum::body::Body;
use axum::http::Method;
use common::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

fn fingerprint(root: &Path) -> BTreeMap<String, (u128, String)> {
    walkdir::WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .map(Result::unwrap)
        .map(|e| {
            let m = e.metadata().unwrap();
            let what = if e.file_type().is_file() {
                vem_core::hash::sha256_file(e.path()).unwrap().0
            } else {
                "dir".into()
            };
            (
                e.path().strip_prefix(root).unwrap().display().to_string(),
                (
                    m.modified()
                        .unwrap()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    what,
                ),
            )
        })
        .collect()
}

#[tokio::test]
async fn examiner_walkthrough() {
    let before = (
        fingerprint(&fixture("basic")),
        fingerprint(&fixture("exposure")),
    );
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let audits = audit_count(&dir);

    // Find the exposure session and its Bash call.
    let (_, sessions) = get_json(&app, "/api/sessions?project=bob").await;
    let sid = sessions[0]["id"].as_i64().unwrap();
    let (_, msgs) = get_json(&app, &format!("/api/sessions/{sid}/messages")).await;
    let bash = msgs
        .as_array()
        .unwrap()
        .iter()
        .find(|m| {
            m["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["payload"]["name"] == "Bash")
        })
        .unwrap();
    let mid = bash["id"].as_i64().unwrap();
    let (_, detail) = get_json(&app, &format!("/api/messages/{mid}")).await;
    let kinds: Vec<&str> = detail["observations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"command_executed") && kinds.contains(&"secret_candidate"),
        "{kinds:?}"
    );

    // Raw bytes are verified; annotate the message; the drawer sees it.
    let (_, raw) = get_json(
        &app,
        &format!("/api/provenance/{}/raw", bash["provenance_id"]),
    )
    .await;
    assert_eq!(raw["verified"], true);
    let (s, _) = post_json(&app, "/api/annotations", json!({ "target_type": "message", "target_id": mid, "kind": "tag", "value": "credential exposure" })).await;
    assert_eq!(s, 201);
    let (_, detail) = get_json(&app, &format!("/api/messages/{mid}")).await;
    assert_eq!(detail["annotations"][0]["value"], "credential exposure");

    // Indicators list all five exposure secrets plus the basic one.
    let (_, secrets) = get_json(&app, "/api/activity/indicators?kind=secret_candidate").await;
    assert_eq!(secrets.as_array().unwrap().len(), 6);

    // Search reaches tool input; export the session, download it, count lines.
    let (_, hits) = get_json(&app, "/api/search?q=gh%20auth%20status").await;
    assert_eq!(hits[0]["session_id"], sid);
    let (_, report) = post_json(
        &app,
        "/api/exports",
        json!({ "format": "timesketch-jsonl", "scope": { "kind": "session", "id": sid } }),
    )
    .await;
    let (_, _, body) = send(
        &app,
        req(
            Method::GET,
            &format!("/api/exports/{}", report["name"].as_str().unwrap()),
        )
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(
        String::from_utf8(body).unwrap().lines().count() as u64,
        report["events"].as_u64().unwrap()
    );

    // Audit: one annotation, one export. Evidence untouched.
    let (_, audit) = get_json(&app, "/api/audit").await;
    let tail: Vec<&str> = audit.as_array().unwrap()[audits as usize..]
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    assert_eq!(tail, vec!["annotation.create", "export"]);
    assert_eq!(
        (
            fingerprint(&fixture("basic")),
            fingerprint(&fixture("exposure"))
        ),
        before
    );

    // The SPA is served: the real build if present, else the not-built page.
    let (s, _, page) = send(
        &app,
        req(Method::GET, "/sessions/1").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(s, 200);
    let page = String::from_utf8(page).unwrap();
    if Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../frontend/dist/index.html")
        .exists()
    {
        assert!(page.contains("id=\"root\""));
    } else {
        assert!(page.contains("not built"));
    }
}
