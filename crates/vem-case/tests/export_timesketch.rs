mod common;

use common::*;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::timesketch::{write_csv, write_jsonl};
use vem_case::export::{events, Scope};
use vem_case::ingest::ingest;
use vem_case::query::{sessions, SessionFilter};
use vem_case::Case;

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &fixture_root(), AttachOptions { label: "alice".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    (tmp, case)
}

#[test]
fn one_event_per_message_tool_call_and_observation() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    assert_eq!(ev.len(), 22 + 4 + 6);
    let first = ev.iter().find(|e| e.attributes.get("record_type").map(String::as_str) == Some("ai-title")).unwrap();
    assert_eq!(first.datetime, None);
    assert_eq!(first.timestamp_desc, "No Timestamp");
    assert_eq!(first.source, "AI:CLAUDE_CODE");
    assert_eq!(first.source_long, "claude_code:message:meta");
    assert!(first.display_name.starts_with("alice:projects/"));
    assert!(first.tags.contains(&"meta".to_string()) && first.tags.contains(&"absent".to_string()));
    assert_eq!(first.provenance.byte_offset, 0);
    let cmd = ev.iter().find(|e| e.attributes.get("observation_kind").map(String::as_str) == Some("command_executed")).unwrap();
    assert_eq!(cmd.datetime.as_deref(), Some("2026-09-30T10:00:02.000Z"));
    assert_eq!(cmd.timestamp_desc, "Observation Time (stored)");
    assert_eq!(cmd.attributes["command"], "ls -la");
    assert!(cmd.message.contains("ls -la"));
    let tc = ev.iter().find(|e| e.attributes.get("tool_name").map(String::as_str) == Some("Bash")).unwrap();
    assert_eq!(tc.timestamp_desc, "Tool Call Started (stored)");
    assert_eq!(tc.source_long, "claude_code:tool_call:shell");
    let user = ev.iter().find(|e| e.attributes.get("role").map(String::as_str) == Some("user") && e.message.contains("Create a notes file")).unwrap();
    assert_eq!(user.message, "[claude-code] user: Create a notes file and list the repo");
    assert_eq!(user.timestamp_desc, "Message Timestamp (stored)");
    assert!(ev.windows(2).all(|w| w[0].datetime.is_none() || w[1].datetime.is_none() || w[0].datetime <= w[1].datetime), "sorted by time");
}

#[test]
fn scopes_restrict_events() {
    let (_t, case) = ingested();
    let s0 = sessions(&case, &SessionFilter::default()).unwrap().into_iter().find(|s| s.harness_session_id == S0).unwrap();
    let ev = events(&case, &Scope::Session(s0.id)).unwrap();
    assert_eq!(ev.len(), 2);
    let ev = events(&case, &Scope::Root(s0.root_id)).unwrap();
    assert_eq!(ev.len(), 32);
}

#[test]
fn jsonl_lines_carry_timesketch_fields_and_attributes_flattened() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    let mut buf = Vec::new();
    write_jsonl(&ev, &mut buf).unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(buf).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), ev.len());
    let with_time = lines.iter().find(|l| !l["datetime"].is_null()).unwrap();
    for key in ["datetime", "timestamp_desc", "message", "source", "source_long", "display_name", "tags", "evidence_file_sha256", "evidence_byte_offset", "evidence_record_sha256"] {
        assert!(with_time.get(key).is_some(), "missing {key}");
    }
    assert!(with_time["tags"].is_array());
    assert!(with_time["evidence_byte_offset"].is_string(), "attributes are strings");
}

#[test]
fn csv_has_union_header_and_one_row_per_event() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    let mut buf = Vec::new();
    write_csv(&ev, &mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();
    let mut rdr = csv::Reader::from_reader(text.as_bytes());
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(String::from).collect();
    assert_eq!(&headers[..7], &["datetime", "timestamp_desc", "message", "source", "source_long", "display_name", "tag"]);
    assert!(headers.contains(&"command".to_string()));
    let rows: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
    assert_eq!(rows.len(), ev.len());
    assert!(rows.iter().all(|r| r.len() == headers.len()));
    let tagged = rows.iter().find(|r| r[6].contains('|')).expect("pipe-joined tags");
    assert!(tagged[6].contains("claude-code"));
}
