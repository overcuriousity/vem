mod common;

use common::*;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::testing::VecSink;

#[test]
fn pairs_tool_uses_with_results() {
    let sink = parse_fixture(S1_FILE);
    assert_eq!(sink.tool_calls.len(), 4);
    let names: Vec<&str> = sink.tool_calls.iter().map(|(_, _, t)| t.name.as_str()).collect();
    assert_eq!(names, vec!["Bash", "Write", "Edit", "Agent"]);
    let cats: Vec<ToolCategory> = sink.tool_calls.iter().map(|(_, _, t)| t.category).collect();
    assert_eq!(cats, vec![ToolCategory::Shell, ToolCategory::FileWrite, ToolCategory::FileEdit, ToolCategory::Agent]);
    let (_, _, bash) = &sink.tool_calls[0];
    assert!(bash.tool_result.is_some());
    assert_eq!(bash.result_text.as_deref(), Some("README.md\nsrc\n"));
    assert_eq!(bash.input["command"], "ls -la");
    assert!(!bash.is_error);
    assert_eq!(bash.started.value.as_deref(), Some("2026-09-30T10:00:01.000Z"));
    assert_eq!(bash.ended.value.as_deref(), Some("2026-09-30T10:00:02.000Z"));
    assert_eq!(bash.result_payload.as_ref().unwrap()["stdout"], "README.md\nsrc\n");
    let (_, _, agent) = &sink.tool_calls[3];
    assert_eq!(agent.result_text.as_deref(), Some("Found settings.json"));
    assert!(sink.anomalies_of(AnomalyKind::UnpairedToolResult).is_empty());
}

#[test]
fn derives_command_file_and_subagent_observations() {
    let sink = parse_fixture(S1_FILE);
    let cmd = sink.observations_of(ObservationKind::CommandExecuted);
    assert_eq!(cmd.len(), 1);
    assert_eq!(cmd[0].command.as_deref(), Some("ls -la"));
    assert_eq!(cmd[0].confidence, Confidence::High);
    assert_eq!(cmd[0].timestamp.value.as_deref(), Some("2026-09-30T10:00:02.000Z"));
    assert!(matches!(cmd[0].derived_from, Derivation::ToolCall(ToolCallHandle(1))));

    let written = sink.observations_of(ObservationKind::FileWritten);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].path.as_deref(), Some("/home/alice/proj/notes.md"));
    assert_eq!(written[0].after_blob.as_deref(), Some(sha256_hex(b"# Notes\n").as_str()));
    assert_eq!(written[0].before_blob, None);
    assert_eq!(sink.blobs.get(&sha256_hex(b"# Notes\n")).unwrap(), b"# Notes\n");

    let edited: Vec<&ObservationDraft> = sink
        .observations_of(ObservationKind::FileEdited)
        .into_iter()
        .filter(|o| matches!(o.derived_from, Derivation::ToolCall(_)))
        .collect();
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0].before_blob.as_deref(), Some(sha256_hex(b"# Notes\n").as_str()));
    assert_eq!(edited[0].after_blob.as_deref(), Some(sha256_hex(b"# Notes\n\n- first\n").as_str()));
    assert_eq!(edited[0].details["replaceAll"], false);
    assert!(edited[0].details["structuredPatch"].is_array());

    let spawned = sink.observations_of(ObservationKind::SubagentSpawned);
    assert_eq!(spawned.len(), 1);
    assert_eq!(spawned[0].details["subagent_type"], "Explore");
    assert_eq!(spawned[0].details["agent_id"], "0123456789abcdef");
}

#[test]
fn categorizes_known_and_mcp_tools() {
    use vem_adapters::claude_code::tools::categorize;
    assert_eq!(categorize("Bash"), ToolCategory::Shell);
    assert_eq!(categorize("Read"), ToolCategory::FileRead);
    assert_eq!(categorize("Glob"), ToolCategory::Search);
    assert_eq!(categorize("WebFetch"), ToolCategory::Web);
    assert_eq!(categorize("mcp__github__list_issues"), ToolCategory::Mcp);
    assert_eq!(categorize("Whatever"), ToolCategory::Other);
}

#[test]
fn unpaired_result_is_an_anomaly_and_unfinished_use_is_a_result_less_call() {
    let (tmp, rel) = temp_root_with_transcript(
        "dddddddd-0000-4000-8000-00000000000d",
        &[
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_elsewhere","content":"x"}]},"uuid":"q1","timestamp":"2026-09-30T10:00:00Z","sessionId":"dddddddd-0000-4000-8000-00000000000d"}"#,
            r#"{"type":"assistant","message":{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"toolu_open","name":"Bash","input":{"command":"sleep 999"}}]},"uuid":"q2","timestamp":"2026-09-30T10:00:01Z","sessionId":"dddddddd-0000-4000-8000-00000000000d"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    let a = sink.anomalies_of(AnomalyKind::UnpairedToolResult);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].details["tool_use_id"], "toolu_elsewhere");
    assert_eq!(sink.tool_calls.len(), 1);
    let (_, _, open) = &sink.tool_calls[0];
    assert_eq!(open.name, "Bash");
    assert!(open.tool_result.is_none());
    assert_eq!(open.ended, Timestamp::absent());
    assert_eq!(sink.observations_of(ObservationKind::CommandExecuted).len(), 1, "a command we saw issued is still an observation");
}

#[test]
fn read_glob_web_and_multiedit_observations() {
    let (tmp, rel) = temp_root_with_transcript(
        "eeeeeeee-0000-4000-8000-00000000000e",
        &[
            r#"{"type":"assistant","message":{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/p/a.txt"}},{"type":"tool_use","id":"t2","name":"Grep","input":{"pattern":"TODO","path":"/p"}},{"type":"tool_use","id":"t3","name":"WebFetch","input":{"url":"https://example.org/x","prompt":"summarize"}},{"type":"tool_use","id":"t4","name":"MultiEdit","input":{"file_path":"/p/b.txt","edits":[{"old_string":"a","new_string":"b"},{"old_string":"c","new_string":"d"}]}}]},"uuid":"r1","timestamp":"2026-09-30T10:00:01Z","sessionId":"eeeeeeee-0000-4000-8000-00000000000e"}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"1\thello"},{"type":"tool_result","tool_use_id":"t2","content":"/p/a.txt:1:TODO"},{"type":"tool_result","tool_use_id":"t3","content":"page text"},{"type":"tool_result","tool_use_id":"t4","content":"ok"}]},"uuid":"r2","timestamp":"2026-09-30T10:00:02Z","sessionId":"eeeeeeee-0000-4000-8000-00000000000e"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.tool_calls.len(), 4);
    let reads = sink.observations_of(ObservationKind::FileRead);
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0].path.as_deref(), Some("/p/a.txt"));
    assert_eq!(reads[1].path.as_deref(), Some("/p"));
    assert_eq!(reads[1].confidence, Confidence::Medium);
    assert_eq!(reads[1].details["pattern"], "TODO");
    let urls = sink.observations_of(ObservationKind::UrlReferenced);
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0].path.as_deref(), Some("https://example.org/x"));
    let edits = sink.observations_of(ObservationKind::FileEdited);
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[1].before_blob.as_deref(), Some(sha256_hex(b"c").as_str()));
    assert_eq!(edits[1].after_blob.as_deref(), Some(sha256_hex(b"d").as_str()));
}
