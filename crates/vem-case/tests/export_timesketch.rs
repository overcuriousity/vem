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
    attach(
        &mut case,
        &fixture_root(),
        AttachOptions {
            label: "alice".into(),
            host: None,
            user: None,
            os: None,
            harness: None,
            retain: true,
        },
    )
    .unwrap();
    ingest(&mut case, None).unwrap();
    (tmp, case)
}

#[test]
fn one_event_per_message_tool_call_and_observation() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    assert_eq!(ev.len(), 23 + 4 + 6);
    // A record without a timestamp is placed at its session's start, labelled inferred (I5).
    let first = ev
        .iter()
        .find(|e| e.attributes.get("record_type").map(String::as_str) == Some("ai-title"))
        .unwrap();
    assert_eq!(
        first.datetime.as_deref(),
        Some("2026-09-30T10:00:00.000Z"),
        "S1's first_ts"
    );
    assert_eq!(
        first.timestamp_desc,
        vem_case::export::events::SESSION_START_INFERRED
    );
    assert!(first.tags.contains(&"inferred".to_string()));
    assert_eq!(
        first.attributes["ts_origin"], "absent",
        "the record's own origin is kept"
    );
    assert_eq!(first.source, "AI:CLAUDE_CODE");
    assert_eq!(first.source_long, "claude_code:message:meta");
    assert!(first.display_name.starts_with("alice:projects/"));
    assert!(first.tags.contains(&"meta".to_string()) && first.tags.contains(&"absent".to_string()));
    assert_eq!(first.provenance.byte_offset, 0);
    let cmd = ev
        .iter()
        .find(|e| {
            e.attributes.get("observation_kind").map(String::as_str) == Some("command_executed")
        })
        .unwrap();
    assert_eq!(
        cmd.datetime.as_deref(),
        Some("2026-09-30T10:00:01.000Z"),
        "tool_use time (M6)"
    );
    assert_eq!(cmd.timestamp_desc, "Observation Time (stored)");
    assert_eq!(cmd.attributes["command"], "ls -la");
    assert!(cmd.message.contains("ls -la"));
    let tc = ev
        .iter()
        .find(|e| e.attributes.get("tool_name").map(String::as_str) == Some("Bash"))
        .unwrap();
    assert_eq!(tc.timestamp_desc, "Tool Call Started (stored)");
    assert_eq!(tc.source_long, "claude_code:tool_call:shell");
    let user = ev
        .iter()
        .find(|e| {
            e.attributes.get("role").map(String::as_str) == Some("user")
                && e.message.contains("Create a notes file")
        })
        .unwrap();
    assert_eq!(
        user.message,
        "[claude-code] user: Create a notes file and list the repo"
    );
    assert_eq!(user.timestamp_desc, "Message Timestamp (stored)");
    assert!(
        ev.windows(2).all(|w| w[0].datetime.is_none()
            || w[1].datetime.is_none()
            || w[0].datetime <= w[1].datetime),
        "sorted by time"
    );
}

#[test]
fn scopes_restrict_events() {
    let (_t, case) = ingested();
    let s0 = sessions(&case, &SessionFilter::default())
        .unwrap()
        .into_iter()
        .find(|s| s.harness_session_id == S0)
        .unwrap();
    let ev = events(&case, &Scope::Session(s0.id)).unwrap();
    assert_eq!(ev.len(), 2);
    let ev = events(&case, &Scope::Root(s0.root_id)).unwrap();
    assert_eq!(ev.len(), 33);
}

#[test]
fn jsonl_lines_carry_timesketch_fields_and_attributes_flattened() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    let mut buf = Vec::new();
    write_jsonl(&ev, &mut buf).unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(buf)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), ev.len());
    let with_time = lines.iter().find(|l| !l["datetime"].is_null()).unwrap();
    for key in [
        "datetime",
        "timestamp_desc",
        "message",
        "source",
        "source_long",
        "display_name",
        "tag",
        "evidence_file_sha256",
        "evidence_byte_offset",
        "evidence_record_sha256",
    ] {
        assert!(with_time.get(key).is_some(), "missing {key}");
    }
    assert!(
        with_time.get("tags").is_none(),
        "Timesketch reads `tag`, not `tags`"
    );
    assert!(with_time["tag"].is_array());
    assert!(
        with_time["evidence_byte_offset"].is_string(),
        "attributes are strings"
    );
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
    assert_eq!(
        &headers[..7],
        &[
            "datetime",
            "timestamp_desc",
            "message",
            "source",
            "source_long",
            "display_name",
            "tag"
        ]
    );
    assert!(headers.contains(&"command".to_string()));
    let rows: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
    assert_eq!(rows.len(), ev.len());
    assert!(rows.iter().all(|r| r.len() == headers.len()));
    let tagged = rows
        .iter()
        .find(|r| r[6].starts_with("claude-code,"))
        .expect("comma-separated tags");
    assert!(tagged[6].split(',').count() >= 3, "{}", &tagged[6]);
    assert!(!tagged[6].contains('|'));
}

/// Mirrors the rules of Timesketch's importer (`read_and_validate_jsonl` / `read_and_validate_csv`):
/// `message`, `datetime` and `timestamp_desc` are mandatory, `datetime` must parse, rows without one are
/// dropped, and `tag` is a list (JSONL) or a comma-separated string (CSV).
#[test]
fn every_event_survives_timesketch_validation() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    let mut buf = Vec::new();
    write_jsonl(&ev, &mut buf).unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(buf)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for l in &lines {
        for key in ["message", "datetime", "timestamp_desc"] {
            assert!(
                l[key].is_string() && !l[key].as_str().unwrap().is_empty(),
                "{key} missing in {l}"
            );
        }
        assert!(
            chrono::DateTime::parse_from_rfc3339(l["datetime"].as_str().unwrap()).is_ok(),
            "{l}"
        );
        assert!(l["tag"].as_array().unwrap().iter().all(|t| t.is_string()));
    }
    let mut buf = Vec::new();
    write_csv(&ev, &mut buf).unwrap();
    let mut rdr = csv::Reader::from_reader(buf.as_slice());
    for r in rdr.records() {
        let r = r.unwrap();
        assert!(
            chrono::DateTime::parse_from_rfc3339(&r[0]).is_ok(),
            "CSV datetime {:?}",
            &r[0]
        );
        assert!(!r[1].is_empty() && !r[2].is_empty());
    }
}
