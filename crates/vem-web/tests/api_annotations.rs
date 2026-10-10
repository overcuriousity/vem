mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn annotations_round_trip_with_audit() {
    let (_t, dir) = ingested(&["basic"]);
    let app = app(&dir);
    let (_, sessions) = get_json(&app, "/api/sessions").await;
    let sid = sessions[0]["id"].as_i64().unwrap();
    let audits = audit_count(&dir);
    let (s, a) = post_json(&app, "/api/annotations", json!({ "target_type": "session", "target_id": sid, "kind": "bookmark", "value": "look here" })).await;
    assert_eq!(s, 201);
    assert_eq!(
        (a["kind"].as_str(), a["target_id"].as_i64()),
        (Some("bookmark"), Some(sid))
    );
    assert_eq!(audit_count(&dir), audits + 1);
    let (_, listed) = get_json(
        &app,
        &format!("/api/annotations?target_type=session&target_id={sid}"),
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let id = a["id"].as_i64().unwrap();
    assert_eq!(delete(&app, &format!("/api/annotations/{id}")).await, 204);
    assert_eq!(audit_count(&dir), audits + 2);
    assert_eq!(delete(&app, &format!("/api/annotations/{id}")).await, 404);
    for bad in [
        json!({ "target_type": "session", "target_id": 999999, "kind": "note", "value": "x" }),
        json!({ "target_type": "planet", "target_id": sid, "kind": "note", "value": "x" }),
        json!({ "target_type": "session", "target_id": sid, "kind": "shout", "value": "x" }),
        json!({ "target_type": "session", "target_id": sid, "kind": "note", "value": "  " }),
    ] {
        let (s, e) = post_json(&app, "/api/annotations", bad.clone()).await;
        assert_eq!(
            (s.as_u16(), e["kind"].as_str()),
            (422, Some("invalid")),
            "{bad}"
        );
    }
    let (s, _) = post_json(
        &app,
        "/api/annotations",
        json!({ "target_type": "session" }),
    )
    .await;
    assert!(s.is_client_error());
    assert_eq!(
        audit_count(&dir),
        audits + 2,
        "refused writes are not audited"
    );
}
