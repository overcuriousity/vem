mod common;

use common::*;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::testing::VecSink;

#[test]
fn parses_primary_session_messages_and_roles() {
    let sink = parse_fixture(S1_FILE);
    assert_eq!(sink.sessions.len(), 1);
    let (_, s) = &sink.sessions[0];
    assert_eq!(s.harness_session_id, S1);
    assert_eq!(s.kind, SessionKind::Primary);
    assert_eq!(sink.messages.len(), 16);
    assert_eq!(sink.messages_with_role(Role::User).len(), 1);
    assert_eq!(sink.messages_with_role(Role::Assistant).len(), 5);
    assert_eq!(sink.messages_with_role(Role::Tool).len(), 4);
    assert_eq!(sink.messages_with_role(Role::System).len(), 1);
    assert_eq!(sink.messages_with_role(Role::Meta).len(), 5);
}

#[test]
fn blocks_carry_kinds_text_and_tool_use_ids() {
    let sink = parse_fixture(S1_FILE);
    let user = &sink.messages_with_role(Role::User)[0];
    assert_eq!(user.blocks.len(), 1);
    assert_eq!(user.blocks[0].kind, BlockKind::Text);
    assert_eq!(
        user.blocks[0].text.as_deref(),
        Some("Create a notes file and list the repo")
    );
    assert_eq!(
        user.harness_uuid.as_deref(),
        Some("u1u1u1u1-0000-4000-8000-000000000001")
    );
    assert_eq!(user.parent_uuid, None);
    assert_eq!(
        user.timestamp.value.as_deref(),
        Some("2026-09-30T10:00:00.000Z")
    );
    assert_eq!(user.timestamp.origin, TsOrigin::Stored);

    let a1 = &sink.messages_with_role(Role::Assistant)[0];
    assert_eq!(a1.model.as_deref(), Some("claude-fable-5-1"));
    assert_eq!(a1.blocks[0].kind, BlockKind::Thinking);
    assert_eq!(a1.blocks[0].text.as_deref(), Some("List first."));
    assert_eq!(a1.blocks[1].kind, BlockKind::ToolUse);
    assert_eq!(a1.blocks[1].tool_use_id.as_deref(), Some("toolu_bash1"));

    let t1 = &sink.messages_with_role(Role::Tool)[0];
    assert_eq!(t1.blocks[0].kind, BlockKind::ToolResult);
    assert_eq!(t1.blocks[0].tool_use_id.as_deref(), Some("toolu_bash1"));
    assert_eq!(t1.blocks[0].text.as_deref(), Some("README.md\nsrc\n"));
    assert!(t1.attributes.contains_key("cwd"));
    assert!(!t1.attributes.contains_key("message"));
    assert!(!t1.attributes.contains_key("toolUseResult"));
}

#[test]
fn meta_records_and_unknown_types_are_kept() {
    let sink = parse_fixture(S1_FILE);
    let meta = sink.messages_with_role(Role::Meta);
    let types: Vec<&str> = meta
        .iter()
        .map(|m| m.harness_record_type.as_str())
        .collect();
    assert_eq!(
        types,
        vec![
            "ai-title",
            "last-prompt",
            "file-history-snapshot",
            "file-history-delta",
            "zz-future-record"
        ]
    );
    let unknown = meta[4];
    assert_eq!(
        unknown.attributes.get("payload").unwrap(),
        &serde_json::json!({"x": 1})
    );
    let anomalies = sink.anomalies_of(AnomalyKind::UnknownRecordType);
    assert_eq!(anomalies.len(), 1);
    assert_eq!(anomalies[0].severity, Severity::Info);
    assert!(anomalies[0].message.contains("zz-future-record"));
}

#[test]
fn truncated_final_line_is_an_anomaly_with_offset() {
    let sink = parse_fixture(S1_FILE);
    let a = sink.anomalies_of(AnomalyKind::TruncatedLine);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].severity, Severity::Warning);
    assert_eq!(a[0].source_file, Some(SourceFileHandle(1)));
    assert!(a[0].byte_offset.unwrap() > 0);
    let prov = a[0]
        .provenance
        .as_ref()
        .expect("record-level anomaly carries provenance");
    assert_eq!(Some(prov.byte_offset), a[0].byte_offset);
    let raw = std::fs::read(fixture_root().join(S1_FILE)).unwrap();
    assert_eq!(
        sha256_hex(&raw[prov.byte_offset as usize..(prov.byte_offset + prov.byte_length) as usize]),
        prov.content_sha256
    );
    assert!(sink.anomalies_of(AnomalyKind::MalformedRecord).is_empty());
}

#[test]
fn provenance_points_at_the_exact_bytes() {
    let sink = parse_fixture(S1_FILE);
    let raw = std::fs::read(fixture_root().join(S1_FILE)).unwrap();
    let first_line_len = raw.iter().position(|&b| b == b'\n').unwrap();
    let (_, _, m0) = &sink.messages[0];
    assert_eq!(m0.provenance.byte_offset, 0);
    assert_eq!(m0.provenance.byte_length, first_line_len as u64);
    assert_eq!(m0.provenance.record_index, 0);
    assert_eq!(
        m0.provenance.content_sha256,
        sha256_hex(&raw[..first_line_len])
    );
    assert_eq!(m0.provenance.parser_name, "claude_code.transcript");
    assert_eq!(m0.provenance.origin, ProvOrigin::Stored);
    for (_, _, m) in &sink.messages {
        let slice = &raw[m.provenance.byte_offset as usize
            ..(m.provenance.byte_offset + m.provenance.byte_length) as usize];
        assert_eq!(sha256_hex(slice), m.provenance.content_sha256);
    }
}

#[test]
fn session_fields_are_learned_from_records() {
    let sink = parse_fixture(S1_FILE);
    let merged = sink
        .updates
        .iter()
        .fold(SessionUpdate::default(), |mut acc, (_, u)| {
            if u.title.is_some() {
                acc.title = u.title.clone();
            }
            if acc.project_path.is_none() {
                acc.project_path = u.project_path.clone();
            }
            if acc.git_branch.is_none() {
                acc.git_branch = u.git_branch.clone();
            }
            if acc.harness_version.is_none() {
                acc.harness_version = u.harness_version.clone();
            }
            if acc.model.is_none() {
                acc.model = u.model.clone();
            }
            acc
        });
    assert_eq!(merged.title.as_deref(), Some("Add notes file"));
    assert_eq!(merged.project_path.as_deref(), Some("/home/alice/proj"));
    assert_eq!(merged.git_branch.as_deref(), Some("main"));
    assert_eq!(merged.harness_version.as_deref(), Some("2.1.294"));
    assert_eq!(merged.model.as_deref(), Some("claude-fable-5-1"));
}

#[test]
fn snapshot_of_all_messages() {
    let sink = parse_fixture(S1_FILE);
    insta::assert_json_snapshot!("s1_messages", sink.messages);
}

#[test]
fn missing_timestamp_on_conversation_record_is_flagged_and_offsets_are_normalized() {
    let (tmp, rel) = temp_root_with_transcript(
        "aaaaaaaa-0000-4000-8000-00000000000a",
        &[
            r#"{"type":"user","message":{"role":"user","content":"no time"},"uuid":"x1","sessionId":"aaaaaaaa-0000-4000-8000-00000000000a"}"#,
            r#"{"type":"user","message":{"role":"user","content":"offset"},"uuid":"x2","timestamp":"2026-09-30T12:00:00.000+02:00","sessionId":"aaaaaaaa-0000-4000-8000-00000000000a"}"#,
            r#"{"type":"user","message":{"role":"user","content":"garbage time"},"uuid":"x3","timestamp":"yesterday","sessionId":"aaaaaaaa-0000-4000-8000-00000000000a"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.messages.len(), 3);
    assert_eq!(sink.messages[0].2.timestamp, Timestamp::absent());
    assert_eq!(
        sink.messages[1].2.timestamp.value.as_deref(),
        Some("2026-09-30T10:00:00.000Z")
    );
    assert_eq!(sink.messages[2].2.timestamp.origin, TsOrigin::Absent);
    assert_eq!(sink.anomalies_of(AnomalyKind::MissingTimestamp).len(), 2);
}

#[test]
fn malformed_terminated_line_is_an_error_anomaly_and_parsing_continues() {
    let (tmp, rel) = temp_root_with_transcript(
        "bbbbbbbb-0000-4000-8000-00000000000b",
        &[
            r#"{"type":"user","message":{"role":"user","content":"ok"},"uuid":"y1","timestamp":"2026-09-30T10:00:00Z","sessionId":"bbbbbbbb-0000-4000-8000-00000000000b"}"#,
            r#"{"type":"user","message":{"role":"user","con"#,
            r#"{"type":"user","message":{"role":"user","content":"after"},"uuid":"y3","timestamp":"2026-09-30T10:00:02Z","sessionId":"bbbbbbbb-0000-4000-8000-00000000000b"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.messages.len(), 2);
    let a = sink.anomalies_of(AnomalyKind::MalformedRecord);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].severity, Severity::Error);
}

#[test]
fn odd_content_shapes_never_panic() {
    let (tmp, rel) = temp_root_with_transcript(
        "cccccccc-0000-4000-8000-00000000000c",
        &[
            r#"{"type":"user","message":{"role":"user"},"uuid":"z1","timestamp":"2026-09-30T10:00:00Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
            r#"{"type":"user","message":{"role":"user","content":[]},"uuid":"z2","timestamp":"2026-09-30T10:00:01Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
            r#"{"type":"user","message":{"role":"user","content":42},"uuid":"z3","timestamp":"2026-09-30T10:00:02Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
            r#"{"type":"assistant","uuid":"z4","timestamp":"2026-09-30T10:00:03Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.messages.len(), 4);
    assert!(sink.messages[0].2.blocks.is_empty());
    assert!(sink.messages[1].2.blocks.is_empty());
    assert_eq!(sink.messages[2].2.blocks[0].kind, BlockKind::Other);
    assert_eq!(sink.messages[2].2.role, Role::User);
    assert_eq!(sink.messages[3].2.role, Role::Assistant);
}

#[test]
fn invalid_utf8_record_is_decoded_lossily_not_lost() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("projects/-p")).unwrap();
    let rel = "projects/-p/99999999-0000-4000-8000-000000000099.jsonl";
    let mut line = br#"{"type":"user","message":{"role":"user","content":"caf"#.to_vec();
    line.push(0xe9); // Latin-1 e-acute: not valid UTF-8
    line.extend_from_slice(br#""},"uuid":"i1","timestamp":"2026-09-30T10:00:00Z","sessionId":"99999999-0000-4000-8000-000000000099"}"#);
    line.push(b'\n');
    std::fs::write(tmp.path().join(rel), &line).unwrap();
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), rel, &mut sink);
    assert_eq!(sink.messages.len(), 1, "the message is kept");
    let m = &sink.messages[0].2;
    assert_eq!(m.blocks[0].text.as_deref(), Some("caf\u{FFFD}"));
    assert_eq!(
        m.provenance.content_sha256,
        sha256_hex(&line[..line.len() - 1]),
        "hash is on the original bytes"
    );
    assert!(sink.anomalies_of(AnomalyKind::MalformedRecord).is_empty());
    let a = sink.anomalies_of(AnomalyKind::InvalidUtf8);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].severity, Severity::Info);
}

/// A transcript whose `file-history-delta` names `backup` for the session's backup file.
fn root_with_delta(backup: &str) -> (tempfile::TempDir, String, String) {
    let sid = "fefefefe-0000-4000-8000-0000000000fe";
    let delta = serde_json::json!({
        "type": "file-history-delta", "trackingPath": "notes.md", "messageId": "m1", "timestamp": "2026-09-30T10:00:00Z",
        "backup": { "backupFileName": backup, "realParentDir": "/home/x", "version": 1 },
        "sessionId": sid,
    });
    let (tmp, rel) = temp_root_with_transcript(sid, &[&delta.to_string()], true);
    std::fs::create_dir_all(tmp.path().join("file-history").join(sid)).unwrap();
    (tmp, rel, sid.to_string())
}

#[test]
fn backup_names_that_leave_the_session_directory_are_not_read() {
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "examiner workstation secret").unwrap();
    let secret_sha = sha256_hex(b"examiner workstation secret");

    let traversal = format!("../../../{}", "secret.txt");
    let absolute = secret.to_string_lossy().to_string();
    for name in [traversal.as_str(), absolute.as_str(), "..", "a\\..\\b"] {
        let (tmp, rel, _) = root_with_delta(name);
        // Make the traversal resolve to the secret if it were followed.
        std::fs::copy(&secret, tmp.path().join("secret.txt")).unwrap();
        let mut sink = VecSink::default();
        parse_file_into(tmp.path(), &rel, &mut sink);
        let obs = sink.observations_of(ObservationKind::FileEdited);
        assert_eq!(obs.len(), 1, "{name}: the observation is kept");
        assert_eq!(obs[0].before_blob, None, "{name}: no before-content");
        assert!(
            !sink.blobs.contains_key(&secret_sha),
            "{name}: the secret was read"
        );
        let a = sink.anomalies_of(AnomalyKind::SuspiciousPath);
        assert_eq!(a.len(), 1, "{name}");
        assert_eq!(a[0].severity, Severity::Warning);
        assert!(a[0].provenance.is_some());
    }

    #[cfg(unix)]
    {
        let (tmp, rel, sid) = root_with_delta("linked@v1");
        std::os::unix::fs::symlink(
            &secret,
            tmp.path().join("file-history").join(&sid).join("linked@v1"),
        )
        .unwrap();
        let mut sink = VecSink::default();
        parse_file_into(tmp.path(), &rel, &mut sink);
        assert_eq!(
            sink.observations_of(ObservationKind::FileEdited)[0].before_blob,
            None
        );
        assert!(
            !sink.blobs.contains_key(&secret_sha),
            "a symlinked backup is not followed"
        );
        assert_eq!(sink.anomalies_of(AnomalyKind::SuspiciousPath).len(), 1);
    }

    // A plain name that simply was not collected is not suspicious.
    let (tmp, rel, _) = root_with_delta("deadbeef@v9");
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert!(sink.anomalies_of(AnomalyKind::SuspiciousPath).is_empty());
}

#[test]
fn backup_path_keeps_the_evidence_separator() {
    use vem_adapters::claude_code::transcript::join_evidence_path;
    assert_eq!(
        join_evidence_path("C:\\proj", "src\\a.rs"),
        "C:\\proj\\a.rs"
    );
    assert_eq!(
        join_evidence_path("/home/alice/proj/", "notes.md"),
        "/home/alice/proj/notes.md"
    );
    assert_eq!(
        join_evidence_path("/home/alice/proj", "sub/notes.md"),
        "/home/alice/proj/notes.md"
    );
}
