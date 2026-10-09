mod common;

use common::*;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::query::*;
use vem_case::Case;

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &fixture_root(), AttachOptions { label: "alice".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    (tmp, case)
}

fn by_hid(case: &Case, hid: &str) -> SessionRow {
    sessions(case, &SessionFilter::default()).unwrap().into_iter().find(|s| s.harness_session_id == hid).unwrap()
}

#[test]
fn lists_roots_stores_and_files() {
    let (_t, case) = ingested();
    let r = roots(&case).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].label, "alice");
    let st = stores(&case, r[0].id).unwrap();
    assert_eq!(st[0].kind, "claude:projects");
    assert!(st[0].file_count >= 6);
    assert!(absent_stores(&case, r[0].id).unwrap().contains(&"claude:todos".to_string()));
    let files = source_files(&case, r[0].id).unwrap();
    assert!(files.iter().all(|f| f.parse_status == "parsed" || f.parse_status == "inventoried"));
    assert_eq!(files.iter().find(|f| f.rel_path == "settings.json").unwrap().parse_status, "inventoried", "no parser reads settings.json");
}

#[test]
fn lists_sessions_with_counts_and_children() {
    let (_t, case) = ingested();
    let all = sessions(&case, &SessionFilter::default()).unwrap();
    assert_eq!(all.len(), 5);
    let order: Vec<&str> = all.iter().map(|s| s.harness_session_id.as_str()).collect();
    assert_eq!(
        order,
        vec!["22222222-0000-4000-8000-000000000002", "agent-0123456789abcdef", S1, "33333333-0000-4000-8000-000000000003", S0],
        "newest first_ts first: orphan 11:00, subagent 10:00:07.2, S1 10:00:00, the sidecar-only session 09-29 16:40, S0 09-29 09:00"
    );
    assert!(sessions(&case, &SessionFilter { project_contains: Some("%".into()), ..Default::default() }).unwrap().is_empty(), "LIKE wildcards are escaped (M13)");
    assert_eq!(sessions(&case, &SessionFilter { project_contains: Some("alice/proj".into()), ..Default::default() }).unwrap().len(), 4);
    let s1 = by_hid(&case, S1);
    assert_eq!(s1.kind, "resumed");
    assert_eq!(s1.message_count, 11);
    assert_eq!(s1.tool_call_count, 4);
    assert_eq!(s1.anomaly_count, 2);
    assert_eq!(s1.child_count, 1);
    assert_eq!(s1.models, vec!["claude-fable-5-1".to_string()]);
    let kids = children(&case, s1.id).unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].kind, "subagent");
    let only_sub = sessions(&case, &SessionFilter { kind: Some("subagent".into()), ..Default::default() }).unwrap();
    assert_eq!(only_sub.len(), 1);
    assert!(session(&case, 999_999).unwrap().is_none());
}

#[test]
fn reads_messages_blocks_tool_calls_observations_claims() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let conv = messages(&case, s1.id, false).unwrap();
    assert_eq!(conv.len(), 11);
    assert_eq!(conv[0].role, "user");
    assert_eq!(conv[0].blocks[0].text.as_deref(), Some("Create a notes file and list the repo"));
    assert_eq!(conv[1].blocks[1].kind, "tool_use");
    assert!(conv[1].blocks[1].tool_call_id.is_some());
    let with_meta = messages(&case, s1.id, true).unwrap();
    assert_eq!(with_meta.len(), 16);
    assert_eq!(with_meta[0].role, "meta");
    let tcs = tool_calls(&case, s1.id).unwrap();
    assert_eq!(tcs.len(), 4);
    assert_eq!(tcs[0].name, "Bash");
    assert_eq!(tcs[0].input["command"], "ls -la");
    let obs = observations(&case, &ObservationFilter { session_id: Some(s1.id), kind: Some("command_executed".into()), ..Default::default() }).unwrap();
    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].command.as_deref(), Some("ls -la"));
    let all_obs = observations(&case, &ObservationFilter::default()).unwrap();
    assert_eq!(all_obs.len(), 6);
    let cl = claims(&case, s1.id).unwrap();
    assert!(cl.iter().any(|c| c.scheme == "claude:origin_session_id" && c.join_status == "matched"));
    let an = anomalies(&case, &AnomalyFilter { severity: Some("warning".into()), ..Default::default() }).unwrap();
    assert!(an.iter().all(|a| a.severity == "warning"));
    assert!(an.iter().any(|a| a.kind == "missing_transcript"));
}

#[test]
fn provenance_and_raw_record_round_trip() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let first = &messages(&case, s1.id, true).unwrap()[0];
    let p = provenance(&case, first.provenance_id).unwrap().unwrap();
    assert_eq!(p.byte_offset, 0);
    assert!(p.rel_path.ends_with("000000000001.jsonl"));
    assert!(p.retained);
    let bytes = raw_record(&case, first.provenance_id).unwrap();
    assert!(bytes.starts_with(b"{\"type\":\"ai-title\""));
    assert_eq!(vem_core::hash::sha256_hex(&bytes), p.content_sha256);
}

#[test]
fn full_text_search_finds_blocks() {
    let (_t, case) = ingested();
    let hits = search(&case, "notes", 10).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.snippet.to_lowercase().contains("notes")));
    assert!(search(&case, "zzzz-nothing-here", 10).unwrap().is_empty());
}

#[test]
fn raw_record_succeeds_for_every_provenance_row() {
    let (_t, case) = ingested();
    let ids: Vec<i64> = {
        let mut stmt = case.conn.prepare("SELECT id FROM provenance ORDER BY id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    assert!(ids.len() > 25, "messages, record observations and anomalies all carry provenance");
    for id in ids {
        let p = provenance(&case, id).unwrap().unwrap();
        let bytes = raw_record(&case, id).unwrap_or_else(|e| panic!("provenance {id} ({} @{}): {e}", p.rel_path, p.byte_offset));
        assert_eq!(bytes.len() as i64, p.byte_length);
    }
    // The subagent meta message cites meta.json itself, not the transcript (C3).
    let sub = by_hid(&case, "agent-0123456789abcdef");
    let meta = messages(&case, sub.id, true).unwrap().into_iter().find(|m| m.harness_record_type == "subagent-meta").unwrap();
    let p = provenance(&case, meta.provenance_id).unwrap().unwrap();
    assert!(p.rel_path.ends_with("agent-0123456789abcdef.meta.json"));
    assert_eq!((p.byte_offset, p.origin.as_str()), (0, "stored"));
    let spawn = claims(&case, sub.id).unwrap().into_iter().find(|c| c.scheme == "claude:spawning_tool_use_id").unwrap();
    assert_eq!(spawn.source_file_id, p.source_file_id);
}

#[test]
fn raw_record_reports_tampering_as_an_integrity_mismatch() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let first = &messages(&case, s1.id, true).unwrap()[0];
    let p = provenance(&case, first.provenance_id).unwrap().unwrap();
    let blob = case.blob_path(&p.file_sha256);
    let mut bytes = std::fs::read(&blob).unwrap();
    bytes[1] ^= 0xff;
    std::fs::write(&blob, bytes).unwrap();
    assert!(matches!(raw_record(&case, first.provenance_id), Err(vem_case::CaseError::IntegrityMismatch(_))));
}
