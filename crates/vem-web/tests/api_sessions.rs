mod common;
use common::*;
use serde_json::Value;

const S1: &str = "0f0f0f0f-0000-4000-8000-000000000001";
const E1: &str = "e1e1e1e1-0000-4000-8000-000000000001";

async fn session_id(app: &axum::Router, hid: &str) -> i64 {
    let (_, all) = get_json(app, "/api/sessions").await;
    all.as_array()
        .unwrap()
        .iter()
        .find(|s| s["harness_session_id"] == hid)
        .unwrap()["id"]
        .as_i64()
        .unwrap()
}

fn hids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s["harness_session_id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn session_list_filters() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let (_, all) = get_json(&app, "/api/sessions").await;
    assert_eq!(all.as_array().unwrap().len(), 7);
    let (_, kids) = get_json(&app, "/api/sessions?has_children=true").await;
    assert_eq!(hids(&kids), vec![S1]);
    let (_, sidecar) = get_json(&app, "/api/sessions?kind=sidecar_only").await;
    assert_eq!(sidecar.as_array().unwrap().len(), 2);
    let (_, bob) = get_json(&app, "/api/sessions?project=bob").await;
    assert_eq!(hids(&bob), vec![E1]);
    let (_, none) = get_json(&app, "/api/sessions?to=2000-01-01").await;
    assert!(none.as_array().unwrap().is_empty());
    let (s, _) = get_json(&app, "/api/sessions?has_children=maybe").await;
    assert!(s.is_client_error());
}

#[tokio::test]
async fn session_detail_messages_tool_calls_observations() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let s1 = session_id(&app, S1).await;
    let (_, d) = get_json(&app, &format!("/api/sessions/{s1}")).await;
    let child = d["children"][0]["id"].as_i64().unwrap();
    let (_, cd) = get_json(&app, &format!("/api/sessions/{child}")).await;
    assert_eq!(cd["ancestors"][0]["id"], s1);
    let (s, e) = get_json(&app, "/api/sessions/999999").await;
    assert_eq!((s.as_u16(), e["kind"].as_str()), (404, Some("not_found")));

    let (_, plain) = get_json(&app, &format!("/api/sessions/{s1}/messages")).await;
    let (_, with_meta) = get_json(&app, &format!("/api/sessions/{s1}/messages?meta=true")).await;
    assert!(plain
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["role"] != "meta"));
    assert!(with_meta.as_array().unwrap().len() >= plain.as_array().unwrap().len());
    assert!(with_meta
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["origin"].is_string() && m["blocks"].is_array()));

    let e1 = session_id(&app, E1).await;
    let (_, tcs) = get_json(&app, &format!("/api/sessions/{e1}/tool-calls")).await;
    assert_eq!(
        tcs.as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Bash", "Edit"]
    );
    let (_, obs) = get_json(&app, &format!("/api/sessions/{e1}/observations")).await;
    assert!(obs
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["kind"] == "upload_detected"));
    let (s, _) = get_json(&app, "/api/sessions/999999/messages").await;
    assert_eq!(s, 404);
}
