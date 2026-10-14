//! Schema v2: `provenance.locator` addresses a record that is not a contiguous byte range (a SQLite row);
//! its bytes are the content blob `content_sha256`.

mod common;

use rusqlite::Connection;
use vem_case::error::CaseError;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::{events, Scope};
use vem_case::ingest::ingest;
use vem_case::query::*;
use vem_case::sink::DbSink;
use vem_case::Case;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::sink::ParseSink;

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(
        &mut case,
        &common::fixture_root(),
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

const VALUE: &[u8] = b"{\"k\":1}";

/// Emits one session with one user message whose provenance is located (`cursorDiskKV:k`), as a
/// SQLite-backed adapter would. Returns (session id, message provenance id).
fn emit_located(case: &Case) -> (i64, i64) {
    let (root_id, store_id, file_id): (i64, i64, i64) = case
        .conn
        .query_row(
            "SELECT root_id, store_id, id FROM source_files ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let mut sink = DbSink::new(&case.conn, &case.dir, root_id, store_id, file_id);
    let sha = sink.blob(VALUE);
    assert_eq!(sha, sha256_hex(VALUE));
    let session = sink.session(SessionDraft {
        harness_session_id: "located-1".into(),
        kind: SessionKind::Primary,
        parent_harness_session_id: None,
        title: None,
        project_path: None,
        git_branch: None,
        harness_version: None,
        first_ts: None,
        last_ts: None,
    });
    sink.message(
        session,
        MessageDraft {
            harness_record_type: "bubble".into(),
            harness_uuid: None,
            parent_uuid: None,
            role: Role::User,
            timestamp: Timestamp {
                value: Some("2026-01-01T00:00:00.000Z".into()),
                origin: TsOrigin::Stored,
            },
            model: None,
            attributes: Default::default(),
            blocks: vec![BlockDraft::text("hello")],
            provenance: Provenance {
                source_file: SourceFileHandle(file_id),
                byte_offset: 0,
                byte_length: VALUE.len() as u64,
                record_index: 0,
                content_sha256: sha,
                parser_name: "test".into(),
                parser_version: "0".into(),
                origin: ProvOrigin::Stored,
                locator: Some("cursorDiskKV:k".into()),
            },
        },
    );
    assert!(sink.error.is_none(), "{:?}", sink.error);
    let session_id: i64 = case
        .conn
        .query_row(
            "SELECT id FROM sessions WHERE harness_session_id = 'located-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let prov: i64 = case
        .conn
        .query_row(
            "SELECT provenance_id FROM messages WHERE session_id = ?1",
            [session_id],
            |r| r.get(0),
        )
        .unwrap();
    (session_id, prov)
}

#[test]
fn migrates_v1_case_to_v2() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("c");
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    {
        let conn = Connection::open(dir.join("case.db")).unwrap();
        conn.execute_batch(&format!(
            "BEGIN;\n{}\nPRAGMA user_version = 1;\nCOMMIT;",
            include_str!("../src/schema.sql")
        ))
        .unwrap();
        let has_locator: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('provenance') WHERE name = 'locator'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_locator, 0, "the v1 DDL has no locator column");
    }
    let case = Case::open(&dir).unwrap();
    let v: i64 = case
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 2);
    let n: i64 = case
        .conn
        .query_row("SELECT count(locator) FROM provenance", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn raw_record_reads_blob_for_located_provenance() {
    let (_tmp, case) = ingested();
    let (_, prov) = emit_located(&case);
    let p = provenance(&case, prov).unwrap().unwrap();
    assert_eq!(p.locator.as_deref(), Some("cursorDiskKV:k"));
    assert_eq!(raw_record(&case, prov).unwrap(), VALUE);

    let blob = vem_case::blobs::path(&case.dir, &sha256_hex(VALUE));
    std::fs::write(&blob, b"{\"k\":2}").unwrap();
    assert!(matches!(
        raw_record(&case, prov),
        Err(CaseError::IntegrityMismatch(_))
    ));
}

#[test]
fn raw_record_unchanged_for_byte_range_provenance() {
    let (_tmp, case) = ingested();
    let (prov, offset): (i64, i64) = case
        .conn
        .query_row(
            "SELECT id, byte_offset FROM provenance WHERE locator IS NULL AND byte_offset > 0 LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(offset > 0);
    let p = provenance(&case, prov).unwrap().unwrap();
    assert!(p.locator.is_none());
    let bytes = raw_record(&case, prov).unwrap();
    assert_eq!(bytes.len() as i64, p.byte_length);
    assert_eq!(sha256_hex(&bytes), p.content_sha256);
}

#[test]
fn export_of_located_record_has_locator_and_no_byte_offset() {
    let (_tmp, case) = ingested();
    let (session_id, _) = emit_located(&case);
    let evs = events(&case, &Scope::Session(session_id)).unwrap();
    assert_eq!(evs.len(), 1);
    let e = &evs[0];
    assert_eq!(
        e.attributes.get("evidence_locator").map(String::as_str),
        Some("cursorDiskKV:k")
    );
    assert!(!e.attributes.contains_key("evidence_byte_offset"));
    assert_eq!(e.provenance.locator.as_deref(), Some("cursorDiskKV:k"));

    // A byte-range record keeps its offset and has no locator.
    let claude = events(&case, &Scope::Case).unwrap();
    let other = claude
        .iter()
        .find(|e| e.attributes.get("session_id").map(String::as_str) != Some("located-1"))
        .unwrap();
    assert!(other.attributes.contains_key("evidence_byte_offset"));
    assert!(!other.attributes.contains_key("evidence_locator"));
    assert!(other.provenance.locator.is_none());
}
