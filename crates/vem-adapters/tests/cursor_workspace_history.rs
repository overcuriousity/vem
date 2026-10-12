//! Cursor IDE workspace storage (composer -> project folder) and local history (`User/History`).

mod cursor_common;
use cursor_common::*;
use serde_json::json;
use vem_core::adapter::ParseOutcome;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::sink::ParseSink;
use vem_core::testing::VecSink;

const GLOBAL: &str = "User/globalStorage/state.vscdb";
const WS: &str = "User/workspaceStorage/ws1/state.vscdb";
const ENTRIES: &str = "User/History/h1/entries.json";
const C1: &str = "1c1c1c1c-0000-4000-8000-000000000001";
const C5: &str = "1c1c1c1c-0000-4000-8000-000000000005";

fn global_then_workspace() -> VecSink {
    let mut sink = VecSink::default();
    let root = fixture("ide");
    parse_ide(&root, GLOBAL, 1, &mut sink);
    let out = parse_ide(&root, WS, 2, &mut sink);
    assert!(matches!(out, ParseOutcome::Parsed { .. }));
    sink
}

#[test]
fn workspace_sets_project_path() {
    let sink = global_then_workspace();
    let s = sink.find_session(C1).unwrap();
    let paths: Vec<_> = sink
        .updates
        .iter()
        .filter(|(h, _)| *h == s)
        .filter_map(|(_, u)| u.project_path.clone())
        .collect();
    assert_eq!(paths, vec!["/home/alice/proj".to_string()]);
    // No new identity claims from workspace storage.
    assert!(sink
        .claims
        .iter()
        .all(|(_, c)| c.source_file == SourceFileHandle(1)));
}

#[test]
fn workspace_json_not_parsed() {
    let mut sink = VecSink::default();
    let out = parse_ide(
        &fixture("ide"),
        "User/workspaceStorage/ws1/workspace.json",
        2,
        &mut sink,
    );
    assert_eq!(out, ParseOutcome::NotParsed);
}

#[test]
fn workspace_composer_without_data_is_missing_transcript() {
    let sink = global_then_workspace();
    let mt: Vec<_> = sink
        .anomalies_of(AnomalyKind::MissingTranscript)
        .into_iter()
        .filter(|a| a.source_file == Some(SourceFileHandle(2)))
        .collect();
    assert_eq!(mt.len(), 1, "{mt:?}");
    assert_eq!(mt[0].severity, Severity::Info);
    assert_eq!(mt[0].details, json!({"composer": C5, "workspace": "ws1"}));
    let p = mt[0].provenance.as_ref().unwrap();
    assert_eq!(
        p.locator.as_deref(),
        Some("ItemTable:composer.composerData")
    );
    assert_eq!(p.parser_name, "cursor-workspace");
    assert_eq!(sha256_hex(&sink.blobs[&p.content_sha256]), p.content_sha256);
}

#[test]
fn file_uri_to_path_decodes() {
    use vem_adapters::cursor::workspace::file_uri_path;
    assert_eq!(
        file_uri_path("file:///home/alice/my%20proj"),
        "/home/alice/my proj"
    );
    assert_eq!(file_uri_path("file:///c%3A/Users/bob"), "c:/Users/bob");
    assert_eq!(
        file_uri_path("vscode-remote://ssh-remote%2Bbox/home/x"),
        "vscode-remote://ssh-remote%2Bbox/home/x"
    );
}

fn history() -> VecSink {
    let mut sink = VecSink::default();
    let out = parse_ide(&fixture("ide"), ENTRIES, 3, &mut sink);
    assert_eq!(out, ParseOutcome::Parsed { records: 2 });
    sink
}

#[test]
fn history_entries_become_local_history_session() {
    let sink = history();
    assert_eq!(sink.sessions.len(), 1);
    let (s, d) = &sink.sessions[0];
    assert_eq!(d.harness_session_id, "history:h1");
    assert_eq!(d.kind, SessionKind::LocalHistory);
    assert_eq!(d.title.as_deref(), Some("/home/alice/proj/a.ts"));
    assert!(d.project_path.is_none());
    assert_eq!(
        d.first_ts.as_ref().unwrap().value.as_deref(),
        Some("2026-01-01T00:00:00.000Z")
    );
    assert_eq!(
        d.last_ts.as_ref().unwrap().value.as_deref(),
        Some("2026-01-01T00:01:00.000Z")
    );
    assert_eq!(sink.messages.len(), 2);
    let file = std::fs::read(fixture("ide").join(ENTRIES)).unwrap();
    for (i, (h, _, m)) in sink.messages.iter().enumerate() {
        assert_eq!(h, s);
        assert_eq!(m.role, Role::Meta);
        assert_eq!(m.harness_record_type, "local-history-entry");
        assert_eq!(m.timestamp.origin, TsOrigin::Stored);
        assert_eq!(m.blocks.len(), 1);
        assert_eq!(m.blocks[0].kind, BlockKind::Attachment);
        let p = &m.provenance;
        assert_eq!(p.record_index, i as u64);
        assert!(p.locator.is_none());
        assert_eq!(p.parser_name, "cursor-history");
        // The provenance is the entry object's exact byte range in entries.json.
        let span = &file[p.byte_offset as usize..(p.byte_offset + p.byte_length) as usize];
        assert_eq!(sha256_hex(span), p.content_sha256);
        let v: serde_json::Value = serde_json::from_slice(span).unwrap();
        assert_eq!(v["id"], m.blocks[0].payload["id"]);
    }
    let first = &sink.messages[0].2.blocks[0].payload;
    assert_eq!(first["source"], "Chat Edit");
    let snap = std::fs::read(fixture("ide").join("User/History/h1/AbCd.ts")).unwrap();
    assert_eq!(sink.blobs[first["blob"].as_str().unwrap()], snap);
}

#[test]
fn history_observations_chain_before_after() {
    let sink = history();
    let obs = sink.observations_of(ObservationKind::FileWritten);
    assert_eq!(obs.len(), 2);
    let a = sha256_hex(&std::fs::read(fixture("ide").join("User/History/h1/AbCd.ts")).unwrap());
    let b = sha256_hex(&std::fs::read(fixture("ide").join("User/History/h1/EfGh.ts")).unwrap());
    assert_eq!(
        (obs[0].before_blob.as_deref(), obs[0].after_blob.as_deref()),
        (None, Some(a.as_str()))
    );
    assert_eq!(
        (obs[1].before_blob.as_deref(), obs[1].after_blob.as_deref()),
        (Some(a.as_str()), Some(b.as_str()))
    );
    for (i, o) in obs.iter().enumerate() {
        assert_eq!(o.path.as_deref(), Some("/home/alice/proj/a.ts"));
        assert_eq!(o.confidence, Confidence::High);
        assert_eq!(
            o.derived_from,
            Derivation::Record(sink.messages[i].2.provenance.clone())
        );
    }
}

/// A copy of the IDE fixture whose `h1/entries.json` is `entries`.
fn with_entries(entries: serde_json::Value) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&fixture("ide"), tmp.path());
    let doc = json!({"version": 1, "resource": "file:///home/alice/proj/a.ts", "entries": entries});
    std::fs::write(
        tmp.path().join(ENTRIES),
        serde_json::to_vec_pretty(&doc).unwrap(),
    )
    .unwrap();
    tmp
}

#[test]
fn history_id_traversal_is_suspicious() {
    let tmp = with_entries(json!([{"id": "../x", "timestamp": 1767225600000u64}]));
    std::fs::write(tmp.path().join("User/History/x"), "outside").unwrap();
    let mut sink = VecSink::default();
    parse_ide(tmp.path(), ENTRIES, 3, &mut sink);
    let sus = sink.anomalies_of(AnomalyKind::SuspiciousPath);
    assert_eq!(sus.len(), 1);
    assert_eq!(sus[0].details["id"], "../x");
    assert!(sus[0].provenance.is_some());
    assert!(sink.blobs.values().all(|b| b != b"outside"));
    let obs = sink.observations_of(ObservationKind::FileWritten);
    assert_eq!(obs.len(), 1);
    assert!(obs[0].after_blob.is_none());
}

#[test]
fn history_missing_snapshot_file_anomaly() {
    let tmp = with_entries(json!([
        {"id": "AbCd.ts", "timestamp": 1767225600000u64},
        {"id": "Gone.ts", "timestamp": 1767225660000u64}
    ]));
    let mut sink = VecSink::default();
    parse_ide(tmp.path(), ENTRIES, 3, &mut sink);
    let mt = sink.anomalies_of(AnomalyKind::MissingTranscript);
    assert_eq!(mt.len(), 1);
    assert_eq!(mt[0].severity, Severity::Info);
    assert_eq!(mt[0].details, json!({"history": "h1", "id": "Gone.ts"}));
    let obs = sink.observations_of(ObservationKind::FileWritten);
    assert_eq!(obs.len(), 2);
    assert!(obs[1].after_blob.is_none());
    assert!(obs[1].before_blob.is_some());
}

#[test]
fn history_snapshot_files_not_parsed() {
    let mut sink = VecSink::default();
    let out = parse_ide(&fixture("ide"), "User/History/h1/AbCd.ts", 3, &mut sink);
    assert_eq!(out, ParseOutcome::NotParsed);
}
