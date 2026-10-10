mod common;

use common::*;
use std::fs::{FileTimes, OpenOptions};
use std::time::{Duration, UNIX_EPOCH};
use vem_case::evidence::{attach, AttachOptions};
use vem_case::Case;

fn opts() -> AttachOptions {
    AttachOptions {
        label: "x".into(),
        host: None,
        user: None,
        os: None,
        harness: None,
        retain: true,
    }
}

/// Attach must record each file's access time as it was before vem read anything: identification reads the
/// head of `history.jsonl`, which would otherwise bump an old atime (relatime updates atime <= mtime).
#[test]
fn atime_is_captured_before_identification_reads_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    copy_dir(&fixture_root(), &root);
    let old = UNIX_EPOCH + Duration::from_secs(978_307_200); // 2001-01-01T00:00:00Z
    let f = OpenOptions::new()
        .write(true)
        .open(root.join("history.jsonl"))
        .unwrap();
    f.set_times(FileTimes::new().set_accessed(old)).unwrap();
    drop(f);
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &root, opts()).unwrap();
    let atime: String = case
        .conn
        .query_row(
            "SELECT atime FROM source_files WHERE rel_path = 'history.jsonl'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(atime, "2001-01-01T00:00:00.000Z");
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_recorded_and_attach_continues() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("claude");
    copy_dir(&fixture_root(), &root);
    let locked = root.join("settings.json");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&locked).is_ok() {
        eprintln!("running as root: permissions are not enforced, skipping");
        return;
    }
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let report = attach(&mut case, &root, opts()).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(report.unreadable, vec!["settings.json".to_string()]);
    assert_eq!(report.file_count, file_count(&root) - 1);
    let (kind, severity, details): (String, String, String) = case
        .conn
        .query_row(
            "SELECT kind, severity, details FROM anomalies WHERE kind = 'unreadable_file'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (kind.as_str(), severity.as_str()),
        ("unreadable_file", "error")
    );
    let d: serde_json::Value = serde_json::from_str(&details).unwrap();
    assert_eq!(d["rel_path"], "settings.json");
    assert!(d["error"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("permission"));
}

#[test]
fn verify_reports_blob_files_missing_from_the_index_without_deleting_them() {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &fixture_root(), opts()).unwrap();
    let stray = case.dir.join("blobs").join("ab".repeat(32));
    std::fs::write(&stray, b"stray").unwrap();
    let report = vem_case::verify::verify(&mut case).unwrap();
    assert_eq!(report.unindexed_blobs, vec!["ab".repeat(32)]);
    assert!(stray.exists(), "verify is read-only on the blob store");
}
