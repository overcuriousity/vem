mod common;

use common::*;
use vem_case::evidence::{attach, child_candidates, AttachOptions};
use vem_case::{Case, CaseError};
use vem_core::hash::sha256_file;
use vem_core::model::Harness;

fn opts(label: &str) -> AttachOptions {
    AttachOptions { label: label.to_string(), host: Some("laptop-1".into()), user: Some("alice".into()), os: Some("linux".into()), harness: None, retain: true }
}

#[test]
fn attaches_fixture_with_manifest_and_retention() {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let before = tree_fingerprint(&fixture_root());
    let report = attach(&mut case, &fixture_root(), opts("alice .claude")).unwrap();
    assert_eq!(tree_fingerprint(&fixture_root()), before, "evidence must not be touched");
    assert_eq!(report.harness, Harness::ClaudeCode);
    assert_eq!(report.stores[0].kind, "claude:projects");
    assert_eq!(report.file_count, file_count(&fixture_root()));
    assert_eq!(report.unclaimed_files, 0);
    assert!(report.absent.contains(&"claude:todos".to_string()));
    assert!(report.unreadable.is_empty());

    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM source_files WHERE root_id = ?1", [report.root_id], |r| r.get(0)).unwrap();
    assert_eq!(n as usize, report.file_count);
    let mut stmt = case.conn.prepare("SELECT rel_path, sha256, size, retained, store_id FROM source_files WHERE root_id = ?1").unwrap();
    let rows: Vec<(String, String, i64, i64, Option<i64>)> = stmt
        .query_map([report.root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for (rel, sha, size, retained, store_id) in &rows {
        let (actual, actual_size) = sha256_file(&fixture_root().join(rel)).unwrap();
        assert_eq!(&actual, sha, "{rel}");
        assert_eq!(actual_size as i64, *size);
        assert_eq!(*retained, 1);
        assert!(case.blob_path(sha).is_file(), "retained copy of {rel}");
        assert!(store_id.is_some(), "{rel} should belong to a store");
    }
    assert!(rows.iter().any(|(rel, ..)| rel == "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl"));
    let (label, harness, ident): (String, String, String) = case
        .conn
        .query_row("SELECT label, harness, identification FROM evidence_roots WHERE id = ?1", [report.root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!(label, "alice .claude");
    assert_eq!(harness, "claude-code");
    assert!(ident.contains("parentUuid"));
    let audits: i64 = case.conn.query_row("SELECT COUNT(*) FROM audit_log WHERE action = 'evidence.attach'", [], |r| r.get(0)).unwrap();
    assert_eq!(audits, 1);
}

#[test]
fn unclaimed_files_are_manifested_without_a_store() {
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    std::fs::write(ev.join("random-note.txt"), "left behind").unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let report = attach(&mut case, &ev, opts("x")).unwrap();
    assert_eq!(report.unclaimed_files, 1);
    let store_id: Option<i64> = case.conn.query_row("SELECT store_id FROM source_files WHERE rel_path = 'random-note.txt'", [], |r| r.get(0)).unwrap();
    assert!(store_id.is_none());
}

#[test]
fn no_retain_skips_copies() {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let mut o = opts("x");
    o.retain = false;
    attach(&mut case, &fixture_root(), o).unwrap();
    let retained: i64 = case.conn.query_row("SELECT COUNT(*) FROM source_files WHERE retained = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(retained, 0);
    assert_eq!(std::fs::read_dir(case.dir.join("blobs")).unwrap().count(), 0);
}

#[test]
fn unrecognized_dir_fails_with_child_hint_and_forced_harness_works() {
    let tmp = tempfile::tempdir().unwrap();
    let collection = tmp.path().join("collection");
    copy_dir(&fixture_root(), &collection.join(".claude"));
    std::fs::create_dir_all(collection.join("unrelated")).unwrap();
    let children = child_candidates(&collection);
    assert_eq!(children.len(), 1);
    assert!(children[0].0.ends_with(".claude"));
    assert_eq!(children[0].1[0].harness, Harness::ClaudeCode);

    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    match attach(&mut case, &collection, opts("x")) {
        Err(CaseError::Unrecognized { hint, .. }) => assert!(hint.contains(".claude"), "{hint}"),
        other => panic!("expected Unrecognized, got {:?}", other.map(|_| ())),
    }
    let roots: i64 = case.conn.query_row("SELECT COUNT(*) FROM evidence_roots", [], |r| r.get(0)).unwrap();
    assert_eq!(roots, 0, "a failed attach leaves nothing behind");

    let mut forced = opts("forced");
    forced.harness = Some(Harness::ClaudeCode);
    let report = attach(&mut case, &collection.join("unrelated"), forced).unwrap();
    assert_eq!(report.harness, Harness::ClaudeCode);
    assert!(report.evidence.iter().any(|e| e.contains("forced")));
    assert_eq!(report.file_count, 0);
}

#[cfg(unix)]
#[test]
fn non_utf8_names_are_stored_losslessly() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    std::fs::write(ev.join(OsStr::from_bytes(b"a\xff.txt")), "one").unwrap();
    std::fs::write(ev.join(OsStr::from_bytes(b"a\xfe.txt")), "two").unwrap();
    std::fs::write(ev.join("a%FF.txt"), "literal percent, valid UTF-8").unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let report = attach(&mut case, &ev, opts("x")).unwrap();
    assert_eq!(report.file_count, file_count(&fixture_root()) + 3, "one odd name must not block the root");
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM anomalies WHERE kind = 'non_utf8_path' AND severity = 'info'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 2);
    let rows: Vec<(String, bool)> = {
        let mut stmt = case.conn.prepare("SELECT rel_path, rel_path_encoded FROM source_files WHERE rel_path LIKE 'a%' ORDER BY rel_path, rel_path_encoded").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|r| r.unwrap()).collect()
    };
    // Distinct names stay distinct, including a valid name that happens to look like an encoding.
    assert_eq!(rows, vec![("a%FE.txt".to_string(), true), ("a%FF.txt".to_string(), false), ("a%FF.txt".to_string(), true)]);
    assert_eq!(vem_case::evidence::decode_rel_path("a%FF.txt", true).as_os_str().as_bytes(), b"a\xff.txt");
    assert_eq!(vem_case::evidence::decode_rel_path("a%FF.txt", false).as_os_str().as_bytes(), b"a%FF.txt");
    let r = vem_case::verify::verify(&mut case).unwrap();
    assert!(r.missing.is_empty() && r.drifted.is_empty(), "{r:?}");
}

#[cfg(unix)]
#[test]
fn symlinks_are_recorded_and_never_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("todo.json"), "outside the evidence root").unwrap();
    std::os::unix::fs::symlink(&outside, ev.join("todos")).unwrap();
    std::os::unix::fs::symlink(outside.join("todo.json"), ev.join("projects/-home-alice-proj/linked.jsonl")).unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let report = attach(&mut case, &ev, opts("x")).unwrap();
    assert_eq!(report.file_count, file_count(&fixture_root()), "nothing outside the root is manifested");
    assert_eq!(report.symlinks, vec!["projects/-home-alice-proj/linked.jsonl".to_string(), "todos".to_string()]);
    let todos = report.stores.iter().find(|s| s.kind == "claude:todos").expect("the linked store is present, not absent");
    assert_eq!(todos.file_count, 0, "and not followed");
    let rows: Vec<(String, String, String, i64)> = {
        let mut stmt = case.conn.prepare("SELECT rel_path, kind, link_target, size FROM source_files WHERE kind = 'symlink' ORDER BY rel_path").unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap().map(|r| r.unwrap()).collect()
    };
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], ("todos".to_string(), "symlink".to_string(), outside.to_string_lossy().to_string(), 0));
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM anomalies WHERE kind = 'symlink_in_evidence'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 2);
    let r = vem_case::verify::verify(&mut case).unwrap();
    assert!(r.missing.is_empty() && r.drifted.is_empty(), "{r:?}");
    let ing = vem_case::ingest::ingest(&mut case, None).unwrap();
    assert_eq!((ing.files_failed, ing.sessions), (0, 5), "the linked transcript is not parsed");
}

#[test]
fn the_case_and_its_exports_stay_out_of_evidence_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    // A case created inside the evidence root.
    let mut inner = Case::create(&ev.join("zcase"), "n", None).unwrap();
    assert!(matches!(attach(&mut inner, &ev, opts("x")), Err(CaseError::EvidenceOverlapsCase { .. })));
    std::fs::remove_dir_all(ev.join("zcase")).unwrap();
    // Evidence inside the case directory.
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    copy_dir(&fixture_root(), &tmp.path().join("c/exports/ev"));
    assert!(matches!(attach(&mut case, &tmp.path().join("c/exports/ev"), opts("x")), Err(CaseError::EvidenceOverlapsCase { .. })));
    // Exports are refused under an attached root, allowed elsewhere.
    attach(&mut case, &ev, opts("x")).unwrap();
    assert!(matches!(vem_case::export::check_output_path(&case, &ev.join("projects/out.jsonl")), Err(CaseError::InsideEvidence { .. })));
    assert!(vem_case::export::check_output_path(&case, &tmp.path().join("out.jsonl")).is_ok());
    // A dangling symlink outside the root that points into it is refused too (NB-2).
    #[cfg(unix)]
    {
        let link = tmp.path().join("link.jsonl");
        std::os::unix::fs::symlink(ev.join("projects/planted.jsonl"), &link).unwrap();
        assert!(matches!(vem_case::export::check_output_path(&case, &link), Err(CaseError::InsideEvidence { .. })));
        let rel = tmp.path().join("rel.jsonl");
        std::os::unix::fs::symlink("ev/projects/planted.jsonl", &rel).unwrap();
        assert!(matches!(vem_case::export::check_output_path(&case, &rel), Err(CaseError::InsideEvidence { .. })));
        assert!(!ev.join("projects/planted.jsonl").exists());
    }
}
