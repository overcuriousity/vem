mod common;

use common::*;
use rusqlite::params;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::Case;

fn attached_case() -> (tempfile::TempDir, Case, i64) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let r = attach(
        &mut case,
        &fixture_root(),
        AttachOptions {
            label: "l".into(),
            host: None,
            user: None,
            os: None,
            harness: None,
            retain: true,
        },
    )
    .unwrap();
    (tmp, case, r.root_id)
}

fn count(case: &Case, sql: &str) -> i64 {
    case.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn session_id(case: &Case, harness_id: &str) -> i64 {
    case.conn
        .query_row(
            "SELECT id FROM sessions WHERE harness_session_id = ?1",
            [harness_id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn ingests_fixture_into_canonical_rows() {
    let (_tmp, mut case, _root) = attached_case();
    let before = tree_fingerprint(&fixture_root());
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(
        tree_fingerprint(&fixture_root()),
        before,
        "evidence must not be touched"
    );
    assert_eq!(report.files_failed, 0);
    assert_eq!(
        (report.files_parsed, report.files_inventoried),
        (6, 4),
        "4 files no parser reads: backup, tool result, snapshot, settings"
    );
    assert_eq!(
        report.sessions, 5,
        "S0, S1, subagent, orphan, and the sidecar-only session of the deleted transcript (I7)"
    );
    assert_eq!(
        report.messages, 23,
        "16 + 2 + (2 + subagent-meta) + 1 + the deleted session's one history prompt (I7)"
    );
    assert_eq!(report.tool_calls, 4);
    assert_eq!(
        report.observations, 7,
        "command, written, edited, spawned, file-history backup, paste, pasted AWS key"
    );
    assert_eq!(
        report.anomalies, 4,
        "truncated line, unknown record type, orphaned file, missing transcript"
    );
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 5);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM messages"), 23);
    assert_eq!(
        count(&case, "SELECT COUNT(*) FROM blocks"),
        count(&case, "SELECT COUNT(*) FROM blocks_fts")
    );
    assert_eq!(
        count(&case, "SELECT COUNT(*) FROM tool_calls"),
        count(&case, "SELECT COUNT(*) FROM tool_calls_fts")
    );
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies WHERE kind = 'truncated_line' AND provenance_id IS NOT NULL"), 1);
    assert_eq!(
        count(&case, "SELECT COUNT(*) FROM anomalies a JOIN sessions s ON s.id = a.session_id WHERE a.kind = 'missing_transcript' AND s.kind = 'sidecar_only'"),
        1,
        "the missing_transcript anomaly links to the sidecar-only session"
    );
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM source_files WHERE parse_status = 'parsed'"
        ),
        6
    );
    assert_eq!(
        count(&case, "SELECT COUNT(*) FROM source_files WHERE parse_status = 'inventoried' AND (rel_path LIKE 'file-history/%' OR rel_path LIKE '%tool-results%' OR rel_path LIKE 'shell-snapshots/%' OR rel_path = 'settings.json')"),
        4
    );
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM stores WHERE kind = 'claude:projects' AND status = 'parsed'"
        ),
        1
    );
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM stores WHERE kind = 'claude:settings' AND status = 'inventoried'"
        ),
        1
    );
    let (records, anomalies): (i64, i64) = case
        .conn
        .query_row("SELECT record_count, anomaly_count FROM source_files WHERE rel_path LIKE '%000000000001.jsonl'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(
        (records, anomalies),
        (17, 2),
        "records read: 16 parsed lines plus the truncated one (M5)"
    );
    let audit: String = case
        .conn
        .query_row(
            "SELECT details FROM audit_log WHERE action = 'ingest'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let audit: serde_json::Value = serde_json::from_str(&audit).unwrap();
    assert_eq!(audit["tool_version"], vem_case::TOOL_VERSION);
    assert!(audit["parsers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p == "claude_code.transcript/1"));
}

#[test]
fn links_subagents_resumptions_and_claims() {
    let (_tmp, mut case, _root) = attached_case();
    ingest(&mut case, None).unwrap();
    let s1 = session_id(&case, S1);
    let s0 = session_id(&case, S0);
    let sub = session_id(&case, "agent-0123456789abcdef");
    let (kind, parent): (String, Option<i64>) = case
        .conn
        .query_row(
            "SELECT kind, parent_session_id FROM sessions WHERE id = ?1",
            [sub],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "subagent");
    assert_eq!(parent, Some(s1));
    let s1_kind: String = case
        .conn
        .query_row("SELECT kind FROM sessions WHERE id = ?1", [s1], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        s1_kind, "resumed",
        "S1 carried session_id of S0 on one record"
    );
    let s0_kind: String = case
        .conn
        .query_row("SELECT kind FROM sessions WHERE id = ?1", [s0], |r| {
            r.get(0)
        })
        .unwrap();
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
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM anomalies WHERE kind = 'unlinked_subagent'"
        ),
        0
    );
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
    let title: Option<String> = case
        .conn
        .query_row("SELECT title FROM sessions WHERE id = ?1", [sub], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(title.as_deref(), Some("Find config files"));
}

#[test]
fn ingest_is_idempotent() {
    let (_tmp, mut case, _root) = attached_case();
    let first = ingest(&mut case, None).unwrap();
    let second = ingest(&mut case, None).unwrap();
    assert_eq!(second.files_parsed, 0);
    assert_eq!(
        second.files_skipped,
        first.files_parsed + first.files_inventoried
    );
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 5);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM messages"), 23);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies"), 4);
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM audit_log WHERE action = 'ingest'"
        ),
        2
    );
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
    assert_eq!(report.sessions, 5, "the real sessions only");
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM sessions WHERE harness_session_id LIKE 'ghost%'"
        ),
        0,
        "a failed file leaves no phantom session (I2)"
    );
    let (status, err): (String, Option<String>) = case
        .conn
        .query_row(
            "SELECT parse_status, parse_error FROM source_files WHERE rel_path LIKE '%ghost%'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "failed");
    assert!(err.unwrap().contains("io error"));
    let again = ingest(&mut case, Some(root)).unwrap();
    assert_eq!(
        (again.files_failed, again.files_parsed),
        (1, 0),
        "failed files are retried on the next ingest"
    );
}

fn attach_copy(retain: bool) -> (tempfile::TempDir, Case, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(
        &mut case,
        &ev,
        AttachOptions {
            label: "l".into(),
            host: None,
            user: None,
            os: None,
            harness: None,
            retain,
        },
    )
    .unwrap();
    (tmp, case, ev)
}

#[test]
fn a_file_changed_after_attach_is_flagged_and_ingested_as_a_new_version() {
    use std::io::Write;
    let (_tmp, mut case, ev) = attach_copy(true);
    let rel = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000000.jsonl";
    std::fs::OpenOptions::new()
        .append(true)
        .open(ev.join(rel))
        .unwrap()
        .write_all(b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"injected after attach\"},\"uuid\":\"inj\",\"timestamp\":\"2026-09-29T09:00:05Z\",\"sessionId\":\"0f0f0f0f-0000-4000-8000-000000000000\"}\n")
        .unwrap();
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(report.files_drifted, 1);
    let rows: Vec<(i64, String)> = {
        let mut stmt = case.conn.prepare("SELECT version, parse_status FROM source_files WHERE rel_path = ?1 ORDER BY version").unwrap();
        stmt.query_map([rel], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(
        rows,
        vec![(1, "superseded".to_string()), (2, "parsed".to_string())],
        "the earlier version is kept"
    );
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM anomalies WHERE kind = 'hash_drift' AND severity = 'error'"
        ),
        1
    );
    // The injected record is attributed to version 2, whose retained copy holds it, so raw bytes verify.
    let (prov_id, file_version): (i64, i64) = case
        .conn
        .query_row(
            "SELECT m.provenance_id, f.version FROM messages m JOIN blocks b ON b.message_id = m.id JOIN provenance p ON p.id = m.provenance_id JOIN source_files f ON f.id = p.source_file_id WHERE b.text = 'injected after attach'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(file_version, 2);
    assert!(vem_case::query::raw_record(&case, prov_id)
        .unwrap()
        .ends_with(b"\"sessionId\":\"0f0f0f0f-0000-4000-8000-000000000000\"}"));
    assert!(
        vem_case::verify::verify(&mut case)
            .unwrap()
            .drifted
            .is_empty(),
        "verify compares against the latest version"
    );
}

#[test]
fn a_detached_case_ingests_from_its_retained_copies() {
    let (_tmp, mut case, ev) = attach_copy(true);
    std::fs::remove_dir_all(&ev).unwrap();
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(
        (report.files_failed, report.sessions, report.messages),
        (0, 5, 23)
    );
}

#[test]
fn an_unavailable_root_fails_cleanly_and_is_retried_when_it_returns() {
    let (tmp, mut case, ev) = attach_copy(false);
    let away = tmp.path().join("away");
    std::fs::rename(&ev, &away).unwrap();
    let first = ingest(&mut case, None).unwrap();
    assert_eq!(first.files_failed, 10);
    assert_eq!(
        count(&case, "SELECT COUNT(*) FROM sessions"),
        0,
        "no phantom sessions"
    );
    assert_eq!(count(&case, "SELECT COUNT(*) FROM source_files WHERE parse_status = 'failed' AND parse_error LIKE 'io error%'"), 10);
    std::fs::rename(&away, &ev).unwrap();
    let second = ingest(&mut case, None).unwrap();
    assert_eq!(
        (
            second.files_failed,
            second.files_parsed,
            second.files_inventoried
        ),
        (0, 6, 4)
    );
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 5);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM messages"), 23);
}

#[cfg(unix)]
#[test]
fn a_transcript_retried_after_a_failure_leaves_no_ghost_sidecar_session() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, mut case, ev) = attach_copy(false);
    let s1 = "0f0f0f0f-0000-4000-8000-000000000001";
    let transcript = ev.join(format!("projects/-home-alice-proj/{s1}.jsonl"));
    std::fs::set_permissions(&transcript, std::fs::Permissions::from_mode(0o000)).unwrap();
    let first = ingest(&mut case, None).unwrap();
    std::fs::set_permissions(&transcript, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        first.files_failed >= 2,
        "the transcript and history.jsonl (NB-1)"
    );
    assert_eq!(
        count(
            &case,
            "SELECT parse_status = 'failed' FROM source_files WHERE rel_path = 'history.jsonl'"
        ),
        1,
        "history waits for the transcript"
    );
    let second = ingest(&mut case, None).unwrap();
    assert_eq!(second.files_failed, 0);
    let ghost = format!(
        "SELECT COUNT(*) FROM sessions WHERE harness_session_id = '{s1}' AND kind = 'sidecar_only'"
    );
    assert_eq!(count(&case, &ghost), 0, "no ghost sidecar_only session");
    assert_eq!(count(&case, &format!("SELECT COUNT(*) FROM anomalies WHERE kind = 'missing_transcript' AND details LIKE '%{s1}%'")), 0);
    assert_eq!(
        count(
            &case,
            &format!("SELECT COUNT(*) FROM sessions WHERE harness_session_id = '{s1}'")
        ),
        1
    );
    let real = session_id(&case, s1);
    let kind: String = case
        .conn
        .query_row("SELECT kind FROM sessions WHERE id = ?1", [real], |r| {
            r.get(0)
        })
        .unwrap();
    // The transcript parser makes it `primary`; linking relabels it `resumed` exactly as in a clean ingest,
    // because S1 carries S0's session id on one record (see links_subagents_resumptions_and_claims).
    assert_eq!(
        kind, "resumed",
        "the real transcript session keeps its clean-ingest kind"
    );
    assert_eq!(
        count(
            &case,
            "SELECT parent_session_id FROM sessions WHERE kind = 'subagent'"
        ),
        real,
        "the subagent's parent is the real session"
    );
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM sessions WHERE kind = 'sidecar_only'"
        ),
        1,
        "only the genuinely deleted session"
    );
}
