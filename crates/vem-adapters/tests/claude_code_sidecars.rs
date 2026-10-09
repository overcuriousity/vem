mod common;

use common::*;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::testing::VecSink;

#[test]
fn file_history_delta_yields_backup_observation_with_before_content() {
    let sink = parse_fixture(S1_FILE);
    let fh: Vec<&ObservationDraft> = sink
        .observations_of(ObservationKind::FileEdited)
        .into_iter()
        .filter(|o| o.details["source"] == "file-history-delta")
        .collect();
    assert_eq!(fh.len(), 1);
    assert_eq!(fh[0].path.as_deref(), Some("/home/alice/proj/notes.md"));
    assert_eq!(fh[0].confidence, Confidence::Medium);
    assert_eq!(fh[0].before_blob.as_deref(), Some(sha256_hex(b"# Notes\n").as_str()));
    assert_eq!(fh[0].details["backupFileName"], "deadbeef00000001@v1");
    assert_eq!(fh[0].timestamp.value.as_deref(), Some("2026-09-30T10:00:04.500Z"));
    assert!(matches!(fh[0].derived_from, Derivation::Record(_)));
}

#[test]
fn history_jsonl_yields_pastes_and_missing_transcripts() {
    let mut sink = VecSink::default();
    parse_file_into(&fixture_root(), S1_FILE, &mut sink);
    parse_file_into(&fixture_root(), "history.jsonl", &mut sink);
    let pastes = sink.observations_of(ObservationKind::PasteDetected);
    assert_eq!(pastes.len(), 1);
    assert_eq!(pastes[0].timestamp.value.as_deref(), Some("2026-10-01T10:01:00.000Z"));
    assert_eq!(pastes[0].details["pastedContents"]["1"]["content"], "AKIAIOSFODNN7EXAMPLE\nline2\nline3");
    assert_eq!(pastes[0].details["display"], "[Pasted text #1 +3 lines] please review");
    let missing = sink.anomalies_of(AnomalyKind::MissingTranscript);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].severity, Severity::Warning);
    assert_eq!(missing[0].details["sessionId"], "33333333-0000-4000-8000-000000000003");
    assert_eq!(missing[0].details["project"], "/home/alice/other");
    let deleted = sink.sessions.iter().find(|(_, s)| s.harness_session_id == "33333333-0000-4000-8000-000000000003").expect("sidecar-only session");
    assert_eq!(deleted.1.kind, SessionKind::SidecarOnly);
    assert_eq!(missing[0].session, Some(deleted.0), "the anomaly links to the sidecar-only session");
}

#[test]
fn every_history_line_of_a_deleted_session_is_kept_with_its_pastes() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("projects")).unwrap();
    let lines = [
        r#"{"display":"first","pastedContents":{"1":{"content":"password0=hunter2"}},"timestamp":1790848800000,"project":"/p","sessionId":"44444444-0000-4000-8000-000000000004"}"#,
        r#"{"display":"second","pastedContents":{"1":{"content":"password1=hunter2"}},"timestamp":1790848801000,"project":"/p","sessionId":"44444444-0000-4000-8000-000000000004"}"#,
        r#"{"display":"third, no paste","pastedContents":{},"timestamp":1790848802000,"project":"/p","sessionId":"44444444-0000-4000-8000-000000000004"}"#,
    ];
    std::fs::write(tmp.path().join("history.jsonl"), lines.join("\n") + "\n").unwrap();
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), "history.jsonl", &mut sink);
    assert_eq!(sink.sessions.len(), 1);
    let (h, s) = &sink.sessions[0];
    assert_eq!(s.kind, SessionKind::SidecarOnly);
    assert_eq!(s.project_path.as_deref(), Some("/p"));
    let prompts: Vec<&str> = sink.messages.iter().map(|(_, _, m)| m.blocks[0].text.as_deref().unwrap()).collect();
    assert_eq!(prompts, vec!["first", "second", "third, no paste"]);
    assert!(sink.messages.iter().all(|(sh, _, m)| sh == h && m.role == Role::User && m.provenance.origin == ProvOrigin::Derived));
    assert_eq!(sink.messages[1].2.attributes["pastedContents"]["1"]["content"], "password1=hunter2");
    let pastes: Vec<&(SessionHandle, ObservationDraft)> = sink.observations.iter().filter(|(_, o)| o.kind == ObservationKind::PasteDetected).collect();
    assert_eq!(pastes.len(), 2);
    assert!(pastes.iter().all(|(sh, _)| sh == h));
    assert_eq!(pastes[1].1.details["pastedContents"]["1"]["content"], "password1=hunter2");
    let missing = sink.anomalies_of(AnomalyKind::MissingTranscript);
    assert_eq!(missing.len(), 1, "one anomaly per deleted session");
    assert_eq!(missing[0].session, Some(*h));
}

#[test]
fn history_timestamps_of_odd_types_do_not_lose_the_entry() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("projects/-p")).unwrap();
    std::fs::write(
        tmp.path().join("projects/-p/abababab-0000-4000-8000-0000000000ab.jsonl"),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"x\"},\"uuid\":\"h1\",\"timestamp\":\"2026-09-30T10:00:00Z\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\",\"parentUuid\":null}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("history.jsonl"),
        concat!(
            "{\"display\":\"a\",\"pastedContents\":{\"1\":{\"content\":\"p\"}},\"timestamp\":\"1790848800000\",\"project\":\"/p\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\"}\n",
            "{\"display\":\"b\",\"pastedContents\":{\"1\":{\"content\":\"q\"}},\"timestamp\":1790848800000.5,\"project\":\"/p\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\"}\n",
            "{\"display\":\"c\",\"pastedContents\":{\"1\":{\"content\":\"r\"}},\"timestamp\":\"soon\",\"project\":\"/p\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\"}\n",
        ),
    )
    .unwrap();
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), "projects/-p/abababab-0000-4000-8000-0000000000ab.jsonl", &mut sink);
    parse_file_into(tmp.path(), "history.jsonl", &mut sink);
    let pastes = sink.observations_of(ObservationKind::PasteDetected);
    assert_eq!(pastes.len(), 3);
    assert_eq!(pastes[0].timestamp.value.as_deref(), Some("2026-10-01T10:00:00.000Z"));
    assert_eq!(pastes[1].timestamp.value.as_deref(), Some("2026-10-01T10:00:00.000Z"));
    assert_eq!(pastes[2].timestamp, Timestamp::absent());
}
