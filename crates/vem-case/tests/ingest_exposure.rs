mod common;

use common::*;
use std::path::{Path, PathBuf};
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::Case;

fn exposure() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code/exposure")
        .canonicalize()
        .unwrap()
}

fn opts(retain: bool) -> AttachOptions {
    AttachOptions {
        label: "bob".into(),
        host: None,
        user: None,
        os: None,
        harness: None,
        retain,
    }
}

fn count(case: &Case, sql: &str) -> i64 {
    case.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn exposure_fixture_ingests_with_paste_and_upload_observations() {
    let before = tree_fingerprint(&exposure());
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &exposure(), opts(true)).unwrap();
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(report.files_failed, 0);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 2);
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM observations WHERE kind = 'paste_detected'"
        ),
        3
    );
    assert_eq!(
        count(
            &case,
            "SELECT COUNT(*) FROM observations WHERE kind = 'upload_detected'"
        ),
        2
    );
    assert_eq!(count(&case, "SELECT COUNT(*) FROM observations WHERE kind = 'file_edited' AND before_blob IS NOT NULL"), 2);
    let unref: Vec<String> = {
        let mut s = case.conn.prepare("SELECT f.rel_path FROM anomalies a JOIN source_files f ON f.id = a.source_file_id WHERE a.kind = 'orphaned_file' AND json_extract(a.details, '$.reason') = 'unreferenced_paste'").unwrap();
        s.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(unref, vec!["paste-cache/0badc0de0badc0de.txt".to_string()]);
    // Idempotent: a second ingest adds nothing.
    let anomalies = count(&case, "SELECT COUNT(*) FROM anomalies");
    ingest(&mut case, None).unwrap();
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies"), anomalies);
    assert_eq!(tree_fingerprint(&exposure()), before, "evidence untouched");
}

#[test]
fn backups_are_read_from_the_retained_copy_when_evidence_is_gone() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    copy_dir(&exposure(), &root);
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &root, opts(true)).unwrap();
    std::fs::remove_dir_all(root.join("file-history")).unwrap();
    std::fs::remove_dir_all(root.join("paste-cache")).unwrap();
    ingest(&mut case, None).unwrap();
    let before: Option<String> = case
        .conn
        .query_row("SELECT before_blob FROM observations WHERE kind = 'file_edited' AND json_extract(details, '$.source') = 'file-history-delta'", [], |r| r.get(0))
        .unwrap();
    let sha = before.expect("backup served from the retained copy");
    assert_eq!(std::fs::read(case.blob_path(&sha)).unwrap(), b"PASSWORD=\n");
    let missing = count(&case, "SELECT COUNT(*) FROM observations o, json_each(o.details, '$.pastes') p WHERE o.kind = 'paste_detected' AND json_extract(p.value, '$.missing') = 1");
    assert_eq!(
        missing, 1,
        "only the paste whose cache file was never collected is missing"
    );
}

const T_OK: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const T_GONE: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

fn transcript_line(sid: &str) -> String {
    format!("{{\"parentUuid\":null,\"isSidechain\":false,\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hi\"}},\"uuid\":\"u-{sid}\",\"timestamp\":\"2026-09-30T10:00:00.000Z\",\"cwd\":\"/x\",\"sessionId\":\"{sid}\",\"version\":\"2.1.0\"}}\n")
}

/// history.jsonl writes a paste blob for its first line, then fails on a line whose transcript failed to
/// parse; the rolled-back file must not leave that blob behind.
#[test]
fn a_failed_parse_leaves_no_orphan_blob_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    std::fs::create_dir_all(root.join("projects/-x")).unwrap();
    std::fs::create_dir_all(root.join("paste-cache")).unwrap();
    std::fs::write(
        root.join(format!("projects/-x/{T_OK}.jsonl")),
        transcript_line(T_OK),
    )
    .unwrap();
    std::fs::write(
        root.join(format!("projects/-x/{T_GONE}.jsonl")),
        transcript_line(T_GONE),
    )
    .unwrap();
    std::fs::write(
        root.join("paste-cache/1234567890abcdef.txt"),
        "pasted bytes unique to this test\n",
    )
    .unwrap();
    std::fs::write(
        root.join("history.jsonl"),
        format!(
            "{{\"display\":\"p\",\"pastedContents\":{{\"1\":{{\"id\":1,\"type\":\"text\",\"contentHash\":\"1234567890abcdef\"}}}},\"timestamp\":1790845200000,\"project\":\"/x\",\"sessionId\":\"{T_OK}\"}}\n{{\"display\":\"q\",\"pastedContents\":{{}},\"timestamp\":1790845200001,\"project\":\"/x\",\"sessionId\":\"{T_GONE}\"}}\n"
        ),
    )
    .unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &root, opts(false)).unwrap();
    std::fs::remove_file(root.join(format!("projects/-x/{T_GONE}.jsonl"))).unwrap(); // not retained: its parse fails
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(
        report.files_failed, 2,
        "the vanished transcript and history.jsonl"
    );
    let indexed = count(&case, "SELECT COUNT(*) FROM blobs");
    let on_disk = std::fs::read_dir(case.dir.join("blobs")).unwrap().count() as i64;
    assert_eq!(on_disk, indexed, "no blob file without an index row");
}
