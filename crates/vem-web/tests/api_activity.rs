mod common;
use base64::Engine;
use common::*;

#[tokio::test]
async fn activity_tabs_link_rows_to_messages() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let (_, cmds) = get_json(&app, "/api/activity/commands").await;
    let cmds = cmds.as_array().unwrap();
    assert!(cmds.iter().any(|r| r["command"] == "ls -la"));
    assert!(cmds.iter().all(|r| r["kind"] == "command_executed"
        && r["message_id"].is_i64()
        && r["harness_session_id"].is_string()));
    let (_, files) = get_json(&app, "/api/activity/files").await;
    assert!(files
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["path"] == "/home/bob/app/deploy.env" && r["before_blob"].is_string()));
    let (_, ups) = get_json(&app, "/api/activity/indicators?kind=upload_detected").await;
    let mut mimes: Vec<&str> = ups
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["details"]["mime"].as_str().unwrap())
        .collect();
    mimes.sort();
    assert_eq!(mimes, vec!["application/pdf", "image/png"]);
    let (s, _) = get_json(&app, "/api/activity/indicators?kind=command_executed").await;
    assert_eq!(s, 422);
    let (s, _) = get_json(&app, "/api/activity/nope").await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn blobs_and_diffs() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let (_, files) = get_json(&app, "/api/activity/files").await;
    let written = files
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "file_written")
        .unwrap()
        .clone();
    let after = written["after_blob"].as_str().unwrap();
    let (_, b) = get_json(&app, &format!("/api/blobs/{after}")).await;
    assert_eq!(
        (
            b["binary"].as_bool(),
            b["text"].as_str(),
            b["truncated"].as_bool()
        ),
        (Some(false), Some("# Notes\n"), Some(false))
    );

    let (_, ups) = get_json(&app, "/api/activity/indicators?kind=upload_detected").await;
    let png = ups
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["details"]["mime"] == "image/png")
        .unwrap()["details"]["content_blob"]
        .as_str()
        .unwrap()
        .to_string();
    let (_, pb) = get_json(&app, &format!("/api/blobs/{png}")).await;
    assert_eq!(pb["binary"], true);
    assert!(pb["text"].is_null());
    let raw = base64::engine::general_purpose::STANDARD
        .decode(pb["bytes_b64"].as_str().unwrap())
        .unwrap();
    assert!(raw.starts_with(b"\x89PNG"));
    assert_eq!(get_json(&app, "/api/blobs/not-a-sha").await.0, 422);
    assert_eq!(
        get_json(&app, &format!("/api/blobs/{}", "0".repeat(64)))
            .await
            .0,
        404
    );

    let edit = files
        .as_array()
        .unwrap()
        .iter()
        .find(|r| {
            r["kind"] == "file_edited"
                && r["after_blob"].is_string()
                && r["path"] == "/home/alice/proj/notes.md"
        })
        .unwrap();
    let (s, d) = get_json(
        &app,
        &format!(
            "/api/diff?before={}&after={}",
            edit["before_blob"].as_str().unwrap(),
            edit["after_blob"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(d["binary"], false);
    let lines = d["hunks"][0]["lines"].as_array().unwrap();
    assert!(lines
        .iter()
        .any(|l| l["tag"] == "insert" && l["text"] == "- first"));
    let (_, created) = get_json(&app, &format!("/api/diff?after={after}")).await;
    assert!(created["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .all(|l| l["tag"] == "insert"));
    let (_, bin) = get_json(&app, &format!("/api/diff?before={after}&after={png}")).await;
    assert_eq!(bin["binary"], true);
    assert_eq!(get_json(&app, "/api/diff").await.0, 422);
    assert_eq!(
        get_json(&app, &format!("/api/diff?before={}", "0".repeat(64)))
            .await
            .0,
        404
    );
}
