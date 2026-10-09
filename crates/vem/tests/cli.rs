use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/claude-code/basic").canonicalize().unwrap()
}

fn vem() -> Command {
    Command::cargo_bin("vem").unwrap()
}

#[test]
fn full_headless_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    let case = tmp.path().join("case1");
    let case_s = case.to_string_lossy().to_string();

    vem().args(["case", "new", &case_s, "--name", "Incident 42", "--examiner", "ex"]).assert().success().stdout(predicate::str::contains("Incident 42"));

    vem().args(["evidence", "add", &case_s, fixture().to_str().unwrap(), "--label", "alice .claude", "--host", "laptop", "--user", "alice", "--os", "linux"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code"))
        .stdout(predicate::str::contains("claude:projects"))
        .stdout(predicate::str::contains("absent"));

    vem().args(["evidence", "list", &case_s]).assert().success().stdout(predicate::str::contains("alice .claude"));

    vem().args(["ingest", &case_s]).assert().success().stdout(predicate::str::contains("sessions: 5")).stdout(predicate::str::contains("anomalies: 4"));

    vem().args(["ingest", &case_s]).assert().success().stdout(predicate::str::contains("files_parsed: 0"));

    vem().args(["inventory", &case_s])
        .assert()
        .success()
        .stdout(predicate::str::contains("truncated_line"))
        .stdout(predicate::str::contains("missing_transcript"))
        .stdout(predicate::str::contains("claude:todos"));

    vem().args(["sessions", &case_s]).assert().success().stdout(predicate::str::contains("0f0f0f0f-0000-4000-8000-000000000001")).stdout(predicate::str::contains("resumed")).stdout(predicate::str::contains("subagent"));

    let sessions_json = vem().args(["--json", "sessions", &case_s]).output().unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&sessions_json.stdout).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 5, "4 transcript sessions and the sidecar-only one");

    let jsonl = tmp.path().join("tl.jsonl");
    vem().args(["export", &case_s, "--format", "timesketch-jsonl", "-o", jsonl.to_str().unwrap()]).assert().success().stdout(predicate::str::contains("33"));
    let text = std::fs::read_to_string(&jsonl).unwrap();
    assert_eq!(text.lines().count(), 33);
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(first.get("timestamp_desc").is_some());

    let csv = tmp.path().join("tl.csv");
    vem().args(["export", &case_s, "--format", "timesketch-csv", "-o", csv.to_str().unwrap()]).assert().success();
    assert!(std::fs::read_to_string(&csv).unwrap().starts_with("datetime,timestamp_desc,message,"));

    let pq = tmp.path().join("tl.parquet");
    vem().args(["export", &case_s, "--format", "vestigo-parquet", "-o", pq.to_str().unwrap()]).assert().success();
    assert!(std::fs::metadata(&pq).unwrap().len() > 0);

    vem().args(["verify", &case_s]).assert().success().stdout(predicate::str::contains("drifted: 0")).stdout(predicate::str::contains("missing: 0"));

    let audits = vem().args(["--json", "inventory", &case_s]).output().unwrap();
    let inv: serde_json::Value = serde_json::from_slice(&audits.stdout).unwrap();
    assert!(inv["audit_log"].as_array().unwrap().iter().any(|a| a["action"] == "export"));
}

#[test]
fn unrecognized_evidence_exits_2_with_hint() {
    let tmp = tempfile::tempdir().unwrap();
    let case = tmp.path().join("case2");
    vem().args(["case", "new", case.to_str().unwrap(), "--name", "n"]).assert().success();
    let collection = tmp.path().join("collection");
    std::fs::create_dir_all(collection.join("unrelated")).unwrap();
    let dot_claude = collection.join(".claude");
    for entry in walkdir::WalkDir::new(fixture()) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(fixture()).unwrap();
        let t = dot_claude.join(rel);
        if entry.file_type().is_dir() { std::fs::create_dir_all(&t).unwrap(); } else { std::fs::create_dir_all(t.parent().unwrap()).unwrap(); std::fs::copy(entry.path(), &t).unwrap(); }
    }
    vem().args(["evidence", "add", case.to_str().unwrap(), collection.to_str().unwrap(), "--label", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(".claude"))
        .stderr(predicate::str::contains("claude-code"));
}
