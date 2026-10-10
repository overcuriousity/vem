mod common;
use common::*;
use serde_json::Value;

#[tokio::test]
async fn case_roots_files_anomalies_audit() {
    let (_t, dir) = ingested(&["basic", "exposure"]);
    let app = app(&dir);
    let (s, case) = get_json(&app, "/api/case").await;
    assert_eq!(s, 200);
    assert_eq!(case["info"]["name"], "web test");
    assert_eq!(case["totals"]["sessions"], 7);

    let (_, roots) = get_json(&app, "/api/roots").await;
    let roots = roots.as_array().unwrap();
    assert_eq!(
        roots
            .iter()
            .map(|r| r["root"]["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["basic", "exposure"]
    );
    assert!(!roots[0]["identification"].as_array().unwrap().is_empty());
    assert!(roots[0]["absent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a == "claude:todos"));
    assert!(roots[0]["ingest"]["counts"]["parsed"].as_i64().unwrap() > 0);

    let id = roots[1]["root"]["id"].as_i64().unwrap();
    let (s, files) = get_json(&app, &format!("/api/roots/{id}/files")).await;
    assert_eq!(s, 200);
    assert!(files
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["sha256"].as_str().unwrap().len() == 64));
    assert!(files
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["rel_path"] == "paste-cache/0badc0de0badc0de.txt"));
    let (s, err) = get_json(&app, "/api/roots/999/files").await;
    assert_eq!((s.as_u16(), err["kind"].as_str()), (404, Some("not_found")));

    let (_, warn) = get_json(&app, "/api/anomalies?severity=warning").await;
    assert!(warn
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["severity"] == "warning"));
    let (_, missing) = get_json(&app, "/api/anomalies?kind=missing_transcript").await;
    assert_eq!(missing.as_array().unwrap().len(), 2);
    let sid = missing[0]["session_id"].as_i64().unwrap();
    let (_, by_session) = get_json(&app, &format!("/api/anomalies?session={sid}")).await;
    assert!(by_session
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["session_id"] == sid));

    let (_, audit) = get_json(&app, "/api/audit").await;
    let actions: Vec<&str> = audit
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions[0], "case.create");
    assert!(actions.contains(&"ingest"));
    let _: &Value = &audit[0]["details"];
}
