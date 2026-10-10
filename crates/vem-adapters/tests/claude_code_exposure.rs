mod common;

use common::*;
use std::path::{Path, PathBuf};
use vem_core::model::*;
use vem_core::sink::ParseSink;
use vem_core::testing::VecSink;

const E1: &str = "e1e1e1e1-0000-4000-8000-000000000001";
const E1_FILE: &str = "projects/-home-bob-app/e1e1e1e1-0000-4000-8000-000000000001.jsonl";

fn exposure() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code/exposure")
        .canonicalize()
        .unwrap()
}

#[test]
fn pastes_resolve_paste_cache_and_flag_missing_files() {
    let root = exposure();
    let mut sink = VecSink::default();
    parse_file_into(&root, E1_FILE, &mut sink);
    parse_file_into(&root, "history.jsonl", &mut sink);
    let pastes = sink.observations_of(ObservationKind::PasteDetected);
    assert_eq!(pastes.len(), 3);
    let p1 = &pastes[0].details["pastes"][0];
    assert_eq!(
        (
            p1["content_hash"].as_str(),
            p1["inline"].as_bool(),
            p1["missing"].as_bool()
        ),
        (Some("a1b2c3d4e5f60718"), Some(false), Some(false))
    );
    let blob = p1["content_blob"].as_str().unwrap();
    assert!(String::from_utf8(sink.blobs[blob].clone())
        .unwrap()
        .starts_with("DATABASE_URL="));
    let p2 = &pastes[1].details["pastes"][0];
    assert_eq!(
        (p2["inline"].as_bool(), p2["missing"].as_bool()),
        (Some(true), Some(false))
    );
    let p3 = &pastes[2].details["pastes"][0];
    assert_eq!(
        (p3["content_hash"].as_str(), p3["missing"].as_bool()),
        (Some("ffffffffffffffff"), Some(true))
    );
}

#[test]
fn a_paste_hash_that_is_not_a_plain_name_is_suspicious() {
    let tmp = tempfile::tempdir().unwrap();
    copy_dir(&exposure(), tmp.path());
    std::fs::write(
        tmp.path().join("history.jsonl"),
        format!("{{\"display\":\"x\",\"pastedContents\":{{\"1\":{{\"id\":1,\"type\":\"text\",\"contentHash\":\"../../etc/passwd\"}}}},\"timestamp\":1790845200000,\"project\":\"/p\",\"sessionId\":\"{E1}\"}}\n"),
    )
    .unwrap();
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), E1_FILE, &mut sink);
    parse_file_into(tmp.path(), "history.jsonl", &mut sink);
    assert_eq!(sink.anomalies_of(AnomalyKind::SuspiciousPath).len(), 1);
    assert_eq!(
        sink.observations_of(ObservationKind::PasteDetected)[0].details["pastes"][0]["missing"],
        true
    );
}

#[test]
fn uploads_attach_to_known_sessions_and_create_sidecar_sessions_for_unknown_ones() {
    let root = exposure();
    let mut sink = VecSink::default();
    parse_file_into(&root, E1_FILE, &mut sink);
    parse_file_into(
        &root,
        "uploads/e1e1e1e1-0000-4000-8000-000000000001/0c11dcd8-shot.png",
        &mut sink,
    );
    parse_file_into(
        &root,
        "uploads/99999999-0000-4000-8000-000000000009/77aa88bb-report.pdf",
        &mut sink,
    );
    let ups: Vec<_> = sink
        .observations
        .iter()
        .filter(|(_, o)| o.kind == ObservationKind::UploadDetected)
        .collect();
    assert_eq!(ups.len(), 2);
    let e1 = sink.find_session(E1).unwrap();
    assert_eq!(ups[0].0, e1);
    assert_eq!(ups[0].1.path.as_deref(), Some("0c11dcd8-shot.png"));
    assert_eq!(ups[0].1.details["mime"], "image/png");
    assert_eq!(ups[0].1.timestamp.origin, TsOrigin::FileMtime);
    assert_eq!(ups[1].1.details["mime"], "application/pdf");
    let unknown = sink.session_draft(ups[1].0);
    assert_eq!(
        (unknown.harness_session_id.as_str(), unknown.kind),
        (
            "99999999-0000-4000-8000-000000000009",
            SessionKind::SidecarOnly
        )
    );
    assert_eq!(sink.anomalies_of(AnomalyKind::MissingTranscript).len(), 1);
    match &ups[0].1.derived_from {
        Derivation::Record(p) => assert_eq!(
            (p.byte_offset, p.parser_name.as_str()),
            (0, "claude_code.uploads")
        ),
        other => panic!("unexpected derivation {other:?}"),
    }
}

#[test]
fn mime_sniffing() {
    use vem_adapters::claude_code::uploads::sniff_mime;
    assert_eq!(sniff_mime(b"\xff\xd8\xff\xe0rest", "x.bin"), "image/jpeg");
    assert_eq!(sniff_mime(b"GIF89a....", "x"), "image/gif");
    assert_eq!(sniff_mime(b"RIFF\0\0\0\0WEBPVP8 ", "x"), "image/webp");
    assert_eq!(sniff_mime(b"hello", "notes.TXT"), "text/plain");
    assert_eq!(sniff_mime(b"\x00\x01", "blob"), "application/octet-stream");
}
