mod common;

use common::*;
use vem_core::model::*;
use vem_core::testing::VecSink;

#[test]
fn subagent_file_is_a_subagent_session_with_parent_and_meta() {
    let sink = parse_fixture(SUB_FILE);
    let (h, s) = &sink.sessions[0];
    assert_eq!(s.kind, SessionKind::Subagent);
    assert_eq!(s.harness_session_id, "agent-0123456789abcdef");
    assert_eq!(s.parent_harness_session_id.as_deref(), Some(S1));
    let title = sink.updates.iter().rev().find_map(|(_, u)| u.title.clone());
    assert_eq!(title.as_deref(), Some("Find config files"));
    let spawn = sink.claims.iter().find(|(_, c)| c.scheme == "claude:spawning_tool_use_id").expect("spawn claim");
    assert_eq!(spawn.1.claimed_id, "toolu_agent1");
    assert_eq!(spawn.1.join_status, JoinStatus::Unmatched);
    assert_eq!(spawn.0, *h);
    assert_eq!(sink.messages.len(), 3, "subagent-meta message, then the two records");
    assert_eq!(sink.messages[0].2.harness_record_type, "subagent-meta");
    assert_eq!(sink.messages[0].2.role, Role::Meta);
    assert_eq!(sink.messages[0].2.provenance.origin, ProvOrigin::Derived);
    assert_eq!(sink.messages[2].2.model.as_deref(), Some("claude-haiku-5-5"));
}

#[test]
fn primary_session_emits_session_id_claims() {
    let sink = parse_fixture(S1_FILE);
    let own = sink.claims.iter().filter(|(_, c)| c.scheme == "claude:sessionId").collect::<Vec<_>>();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].1.claimed_id, S1);
    assert_eq!(own[0].1.join_status, JoinStatus::Matched);
    let origin = sink.claims.iter().filter(|(_, c)| c.scheme == "claude:origin_session_id").collect::<Vec<_>>();
    assert_eq!(origin.len(), 1, "the Edit result carried session_id of S0");
    assert_eq!(origin[0].1.claimed_id, S0);
    assert_eq!(origin[0].1.join_status, JoinStatus::Unmatched);
    assert_eq!(origin[0].1.source_file, SourceFileHandle(1));
}

#[test]
fn orphaned_file_is_flagged_but_parsed() {
    let sink = parse_fixture(ORPHAN_FILE);
    assert_eq!(sink.sessions[0].1.harness_session_id, "22222222-0000-4000-8000-000000000002");
    assert_eq!(sink.messages.len(), 1);
    let a = sink.anomalies_of(AnomalyKind::OrphanedFile);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].severity, Severity::Info);
    assert_eq!(a[0].session, Some(sink.sessions[0].0));
    assert!(a[0].message.contains("orphaned-1759221000000"));
}

#[test]
fn superseded_file_is_flagged() {
    let (tmp, rel) = temp_root_with_transcript(
        "ffffffff-0000-4000-8000-00000000000f",
        &[r#"{"type":"user","message":{"role":"user","content":"hi"},"uuid":"w1","timestamp":"2026-09-30T10:00:00Z","sessionId":"ffffffff-0000-4000-8000-00000000000f"}"#],
        true,
    );
    let from = tmp.path().join(&rel);
    let to = tmp.path().join(format!("{rel}.superseded-1759221000001"));
    std::fs::rename(&from, &to).unwrap();
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &format!("{rel}.superseded-1759221000001"), &mut sink);
    assert_eq!(sink.anomalies_of(AnomalyKind::SupersededFile).len(), 1);
    assert_eq!(sink.sessions[0].1.harness_session_id, "ffffffff-0000-4000-8000-00000000000f");
}

#[test]
fn classify_path_handles_all_shapes() {
    use std::path::Path;
    use vem_adapters::claude_code::transcript::classify_path;
    let p = classify_path(Path::new(S1_FILE)).unwrap();
    assert_eq!((p.session_id.as_str(), p.is_subagent, p.parent_session_id.as_deref(), p.flags.as_slice()), (S1, false, None, &[][..]));
    let s = classify_path(Path::new(SUB_FILE)).unwrap();
    assert_eq!((s.session_id.as_str(), s.is_subagent, s.parent_session_id.as_deref()), ("agent-0123456789abcdef", true, Some(S1)));
    let o = classify_path(Path::new(ORPHAN_FILE)).unwrap();
    assert_eq!(o.flags, vec!["orphaned"]);
    assert!(classify_path(Path::new("projects/x/abc/subagents/agent-1.meta.json")).is_none());
    assert!(classify_path(Path::new("projects/x/abc/tool-results/zz.txt")).is_none());
}
