mod common;

use common::*;
use std::io::Write;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::verify::verify;
use vem_case::Case;

fn case_over_copy() -> (tempfile::TempDir, Case, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &ev, AttachOptions { label: "l".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    (tmp, case, ev)
}

#[test]
fn clean_case_verifies_without_drift() {
    let (tmp, mut case, ev) = case_over_copy();
    let before = tree_fingerprint(&ev);
    let r = verify(&mut case).unwrap();
    let evs = vem_case::export::events(&case, &vem_case::export::Scope::Case).unwrap();
    vem_case::export::parquet::write_parquet(&evs, &tmp.path().join("out.parquet")).unwrap();
    assert_eq!(tree_fingerprint(&ev), before, "verify and export leave the evidence untouched");
    assert!(r.files_checked > 0);
    assert!(r.drifted.is_empty() && r.missing.is_empty() && r.blob_errors.is_empty() && r.roots_unavailable.is_empty());
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM anomalies WHERE kind = 'hash_drift'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn detects_modified_and_missing_evidence_files() {
    let (_tmp, mut case, ev) = case_over_copy();
    std::fs::OpenOptions::new().append(true).open(ev.join("history.jsonl")).unwrap().write_all(b"\n").unwrap();
    std::fs::remove_file(ev.join("settings.json")).unwrap();
    let r = verify(&mut case).unwrap();
    assert_eq!(r.drifted, vec!["history.jsonl".to_string()]);
    assert_eq!(r.missing, vec!["settings.json".to_string()]);
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM anomalies WHERE kind = 'hash_drift'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 2);
}

#[test]
fn detects_corrupted_retained_blob_and_detached_root() {
    let (_tmp, mut case, ev) = case_over_copy();
    let sha: String = case.conn.query_row("SELECT sha256 FROM source_files WHERE rel_path = 'history.jsonl'", [], |r| r.get(0)).unwrap();
    std::fs::write(case.blob_path(&sha), b"corrupted").unwrap();
    std::fs::remove_dir_all(&ev).unwrap();
    let r = verify(&mut case).unwrap();
    assert_eq!(r.roots_unavailable.len(), 1);
    assert_eq!(r.files_checked, 0);
    assert_eq!(r.blob_errors, vec![sha]);
}
