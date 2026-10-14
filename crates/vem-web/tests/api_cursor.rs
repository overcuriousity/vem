//! Cursor sessions over the API: turn keys from `vem_turn`, absorbed tool results, located
//! provenance whose raw bytes are the content blob, and the harness filter.
mod common;
use base64::Engine;
use common::*;
use serde_json::Value;

/// The composer present in both Cursor roots; in the IDE root it is a `cursorDiskKV` session.
const T1: &str = "1c1c1c1c-0000-4000-8000-000000000001";

async fn sessions(app: &axum::Router, query: &str) -> Vec<Value> {
    let (s, v) = get_json(app, &format!("/api/sessions{query}")).await;
    assert!(s.is_success(), "{query}: {s}");
    v.as_array().unwrap().clone()
}

async fn kv_t1(app: &axum::Router) -> i64 {
    sessions(app, "?harness=cursor-ide")
        .await
        .iter()
        .find(|s| s["harness_session_id"] == T1)
        .unwrap()["id"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn cursor_session_messages_have_turn_keys() {
    let (_t, dir) = cursor_case();
    let app = app(&dir);
    let id = kv_t1(&app).await;
    let (_, msgs) = get_json(&app, &format!("/api/sessions/{id}/messages")).await;
    let msgs = msgs.as_array().unwrap();
    let mut turns: Vec<Vec<&Value>> = Vec::new();
    for m in msgs {
        match m["role"].as_str().unwrap() {
            "user" => turns.push(Vec::new()),
            "assistant" | "tool" => turns
                .last_mut()
                .expect("record before the first user bubble")
                .push(m),
            _ => {}
        }
    }
    let mut tools = 0;
    for turn in turns.iter().filter(|t| !t.is_empty()) {
        let key = &turn[0]["turn_key"];
        assert!(key.is_string(), "{turn:?}");
        for m in turn {
            assert_eq!(&m["turn_key"], key, "{m}");
            if m["role"] == "tool" {
                tools += 1;
                assert_eq!(m["content_class"], "tool_result");
                let blocks = m["blocks"].as_array().unwrap();
                assert!(!blocks.is_empty());
                assert!(blocks.iter().all(|b| b["tool_call_id"].is_i64()), "{m}");
            }
        }
    }
    assert!(tools > 0, "fixture has no tool record");
}

#[tokio::test]
async fn raw_of_located_record_is_blob_bytes() {
    let (_t, dir) = cursor_case();
    let app = app(&dir);
    let id = kv_t1(&app).await;
    let (_, msgs) = get_json(&app, &format!("/api/sessions/{id}/messages")).await;
    let first = &msgs.as_array().unwrap()[0];
    let pid = first["provenance_id"].as_i64().unwrap();
    let (_, p) = get_json(&app, &format!("/api/provenance/{pid}")).await;
    assert!(p["locator"].as_str().is_some_and(|l| !l.is_empty()), "{p}");

    let (_, w) = get_json(&app, &format!("/api/provenance/{pid}/raw")).await;
    assert_eq!(w["verified"], true);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(w["bytes_b64"].as_str().unwrap())
        .unwrap();
    let blob = std::fs::read(
        dir.join("blobs")
            .join(p["content_sha256"].as_str().unwrap()),
    )
    .unwrap();
    assert_eq!(bytes, blob);
    let bubble: Value = serde_json::from_slice(&bytes).expect("bubble JSON");
    assert_eq!(bubble["bubbleId"], first["harness_uuid"]);
}

#[tokio::test]
async fn sessions_filter_by_cursor_ide() {
    let (_t, dir) = cursor_case();
    let app = app(&dir);
    let ide = sessions(&app, "?harness=cursor-ide").await;
    assert!(!ide.is_empty());
    assert!(ide.iter().all(|s| s["harness"] == "cursor-ide"));
    let dot = sessions(&app, "?harness=cursor").await;
    assert!(!dot.is_empty());
    assert!(dot.iter().all(|s| s["harness"] == "cursor"));
    let claude = sessions(&app, "?harness=claude-code").await;
    assert!(!claude.is_empty());
    assert!(claude.iter().all(|s| s["harness"] == "claude-code"));
    assert_eq!(
        ide.len() + dot.len() + claude.len(),
        sessions(&app, "").await.len()
    );
}
