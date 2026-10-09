mod common;

use common::*;
use rusqlite::params;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::Case;

fn attached_case() -> (tempfile::TempDir, Case, i64) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let r = attach(&mut case, &fixture_root(), AttachOptions { label: "l".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    (tmp, case, r.root_id)
}

fn count(case: &Case, sql: &str) -> i64 {
    case.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn session_id(case: &Case, harness_id: &str) -> i64 {
    case.conn.query_row("SELECT id FROM sessions WHERE harness_session_id = ?1", [harness_id], |r| r.get(0)).unwrap()
}

#[test]
fn ingests_fixture_into_canonical_rows() {
    let (_tmp, mut case, _root) = attached_case();
    let before = tree_fingerprint(&fixture_root());
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(tree_fingerprint(&fixture_root()), before, "evidence must not be touched");
    assert_eq!(report.files_failed, 0);
    assert_eq!(report.sessions, 4, "S0, S1, subagent, orphan");
    assert_eq!(report.messages, 22, "16 + 2 + (2 + subagent-meta) + 1");
    assert_eq!(report.tool_calls, 4);
    assert_eq!(report.observations, 6, "command, written, edited, spawned, file-history backup, paste");
    assert_eq!(report.anomalies, 4, "truncated line, unknown record type, orphaned file, missing transcript");
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 4);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM messages"), 22);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM blocks"), count(&case, "SELECT COUNT(*) FROM blocks_fts"));
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies WHERE kind = 'truncated_line'"), 1);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies WHERE kind = 'missing_transcript' AND session_id IS NULL"), 1);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM source_files WHERE parse_status = 'parsed'"), count(&case, "SELECT COUNT(*) FROM source_files"));
    assert_eq!(count(&case, "SELECT COUNT(*) FROM stores WHERE kind = 'claude:projects' AND status = 'parsed'"), 1);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM stores WHERE kind = 'claude:settings' AND status = 'inventoried'"), 1);
    let (records, anomalies): (i64, i64) = case
        .conn
        .query_row("SELECT record_count, anomaly_count FROM source_files WHERE rel_path LIKE '%000000000001.jsonl'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((records, anomalies), (16, 2));
}

#[test]
fn links_subagents_resumptions_and_claims() {
    let (_tmp, mut case, _root) = attached_case();
    ingest(&mut case, None).unwrap();
    let s1 = session_id(&case, S1);
    let s0 = session_id(&case, S0);
    let sub = session_id(&case, "agent-0123456789abcdef");
    let (kind, parent): (String, Option<i64>) = case.conn.query_row("SELECT kind, parent_session_id FROM sessions WHERE id = ?1", [sub], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(kind, "subagent");
    assert_eq!(parent, Some(s1));
    let s1_kind: String = case.conn.query_row("SELECT kind FROM sessions WHERE id = ?1", [s1], |r| r.get(0)).unwrap();
    assert_eq!(s1_kind, "resumed", "S1 carried session_id of S0 on one record");
    let s0_kind: String = case.conn.query_row("SELECT kind FROM sessions WHERE id = ?1", [s0], |r| r.get(0)).unwrap();
    assert_eq!(s0_kind, "primary");
    let (status, matched): (String, Option<i64>) = case
        .conn
        .query_row("SELECT join_status, matched_session_id FROM identity_claims WHERE scheme = 'claude:origin_session_id' AND session_id = ?1", [s1], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((status.as_str(), matched), ("matched", Some(s0)));
    let (status, matched): (String, Option<i64>) = case
        .conn
        .query_row("SELECT join_status, matched_session_id FROM identity_claims WHERE scheme = 'claude:spawning_tool_use_id' AND session_id = ?1", [sub], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((status.as_str(), matched), ("matched", Some(s1)));
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies WHERE kind = 'unlinked_subagent'"), 0);
}

#[test]
#[allow(clippy::type_complexity)]
fn finalizes_session_bounds_counts_and_models() {
    let (_tmp, mut case, _root) = attached_case();
    ingest(&mut case, None).unwrap();
    let s1 = session_id(&case, S1);
    let row: (Option<String>, String, Option<String>, String, i64, i64, String, Option<String>, Option<String>) = case
        .conn
        .query_row(
            "SELECT first_ts, first_ts_origin, last_ts, last_ts_origin, message_count, tool_call_count, models, title, project_path FROM sessions WHERE id = ?1",
            [s1],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
        )
        .unwrap();
    assert_eq!(row.0.as_deref(), Some("2026-09-30T10:00:00.000Z"));
    assert_eq!(row.1, "stored");
    assert_eq!(row.2.as_deref(), Some("2026-09-30T10:00:10.100Z"));
    assert_eq!(row.4, 11, "non-meta messages");
    assert_eq!(row.5, 4);
    assert_eq!(row.6, "[\"claude-fable-5-1\"]");
    assert_eq!(row.7.as_deref(), Some("Add notes file"));
    assert_eq!(row.8.as_deref(), Some("/home/alice/proj"));
    let sub = session_id(&case, "agent-0123456789abcdef");
    let title: Option<String> = case.conn.query_row("SELECT title FROM sessions WHERE id = ?1", [sub], |r| r.get(0)).unwrap();
    assert_eq!(title.as_deref(), Some("Find config files"));
}

#[test]
fn ingest_is_idempotent() {
    let (_tmp, mut case, _root) = attached_case();
    let first = ingest(&mut case, None).unwrap();
    let second = ingest(&mut case, None).unwrap();
    assert_eq!(second.files_parsed, 0);
    assert_eq!(second.files_skipped, first.files_parsed);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 4);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM messages"), 22);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies"), 4);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM audit_log WHERE action = 'ingest'"), 2);
}

#[test]
fn unknown_root_filter_is_an_error_and_a_broken_file_is_marked_failed_without_stopping() {
    let (_tmp, mut case, root) = attached_case();
    assert!(ingest(&mut case, Some(root + 99)).is_err());
    // Simulate a file the adapter cannot open: point a source_files row at a path that does not exist.
    case.conn
        .execute(
            "INSERT INTO source_files (root_id, store_id, rel_path, size, sha256, parse_status, version) VALUES (?1, (SELECT id FROM stores WHERE kind = 'claude:projects'), 'projects/-home-alice-proj/ghost-0000-4000-8000-000000000009.jsonl', 0, 'x', 'unparsed', 1)",
            params![root],
        )
        .unwrap();
    let report = ingest(&mut case, Some(root)).unwrap();
    assert_eq!(report.files_failed, 1);
    assert_eq!(report.sessions, 5, "the ghost session row was created before the open failed");
    let (status, err): (String, Option<String>) = case
        .conn
        .query_row("SELECT parse_status, parse_error FROM source_files WHERE rel_path LIKE '%ghost%'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(status, "failed");
    assert!(err.unwrap().contains("io error"));
}
