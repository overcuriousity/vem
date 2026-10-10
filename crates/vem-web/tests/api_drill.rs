mod common;
use base64::Engine;
use common::*;

const E1: &str = "e1e1e1e1-0000-4000-8000-000000000001";

async fn e1_bash_message(app: &axum::Router) -> serde_json::Value {
    let (_, all) = get_json(app, "/api/sessions").await;
    let e1 = all
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["harness_session_id"] == E1)
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (_, msgs) = get_json(app, &format!("/api/sessions/{e1}/messages")).await;
    msgs.as_array()
        .unwrap()
        .iter()
        .find(|m| {
            m["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["kind"] == "tool_use" && b["payload"]["name"] == "Bash")
        })
        .unwrap()
        .clone()
}

#[tokio::test]
async fn message_detail_provenance_and_verified_raw_bytes() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let m = e1_bash_message(&app).await;
    let mid = m["id"].as_i64().unwrap();
    let (s, d) = get_json(&app, &format!("/api/messages/{mid}")).await;
    assert_eq!(s, 200);
    assert_eq!(d["message"]["id"], mid);
    assert_eq!(d["tool_calls"][0]["name"], "Bash");
    assert!(d["observations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["kind"] == "command_executed"));
    assert!(d["annotations"].as_array().unwrap().is_empty());

    let pid = m["provenance_id"].as_i64().unwrap();
    let (_, p) = get_json(&app, &format!("/api/provenance/{pid}")).await;
    assert_eq!(p["parser_name"], "claude_code.transcript");
    assert_eq!(p["content_sha256"].as_str().unwrap().len(), 64);
    let (_, w) = get_json(
        &app,
        &format!("/api/provenance/{pid}/raw?offset=0&len=65536"),
    )
    .await;
    assert_eq!(w["verified"], true);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(w["bytes_b64"].as_str().unwrap())
        .unwrap();
    let expected = vem_case::query::raw_record(&vem_case::Case::open(&dir).unwrap(), pid).unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(w["total_length"], expected.len());
    let (_, past) = get_json(
        &app,
        &format!("/api/provenance/{pid}/raw?offset={}", expected.len() + 10),
    )
    .await;
    assert_eq!(past["bytes_b64"], "");

    for path in [
        "/api/messages/999999",
        "/api/provenance/999999",
        "/api/provenance/999999/raw",
    ] {
        assert_eq!(get_json(&app, path).await.0, 404, "{path}");
    }
}

#[tokio::test]
async fn tampered_retained_bytes_are_an_integrity_conflict() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let m = e1_bash_message(&app).await;
    let pid = m["provenance_id"].as_i64().unwrap();
    let (_, p) = get_json(&app, &format!("/api/provenance/{pid}")).await;
    let blob = dir.join("blobs").join(p["file_sha256"].as_str().unwrap());
    let mut bytes = std::fs::read(&blob).unwrap();
    let at = p["byte_offset"].as_u64().unwrap() as usize + 2;
    bytes[at] ^= 0x01;
    std::fs::write(&blob, bytes).unwrap();
    let (s, e) = get_json(&app, &format!("/api/provenance/{pid}/raw")).await;
    assert_eq!(
        (s.as_u16(), e["kind"].as_str()),
        (409, Some("integrity_mismatch"))
    );
}
