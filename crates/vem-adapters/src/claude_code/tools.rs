//! Tool-use/result pairing and the observations derived from Claude Code tools (spec §6.1).

use super::transcript::{str_field, PendingToolUse, TranscriptState};
use serde_json::{json, Value};
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub fn categorize(name: &str) -> ToolCategory {
    match name {
        "Bash" | "BashOutput" | "KillShell" | "KillBash" => ToolCategory::Shell,
        "Read" | "NotebookRead" => ToolCategory::FileRead,
        "Write" => ToolCategory::FileWrite,
        "Edit" | "MultiEdit" | "NotebookEdit" => ToolCategory::FileEdit,
        "Glob" | "Grep" | "LS" => ToolCategory::Search,
        "WebFetch" | "WebSearch" => ToolCategory::Web,
        "Agent" | "Task" => ToolCategory::Agent,
        n if n.starts_with("mcp__") => ToolCategory::Mcp,
        _ => ToolCategory::Other,
    }
}

fn base(kind: ObservationKind, tool_call: ToolCallHandle, at: &Timestamp) -> ObservationDraft {
    ObservationDraft {
        kind,
        derived_from: Derivation::ToolCall(tool_call),
        path: None,
        command: None,
        before_blob: None,
        after_blob: None,
        timestamp: at.clone(),
        confidence: Confidence::High,
        details: json!({}),
    }
}

fn blob_of(sink: &mut dyn ParseSink, s: Option<String>) -> Option<String> {
    s.map(|c| sink.blob(c.as_bytes()))
}

fn edit_observation(path: Option<String>, old: Option<String>, new: Option<String>, details: Value, tool_call: ToolCallHandle, at: &Timestamp, sink: &mut dyn ParseSink) -> ObservationDraft {
    ObservationDraft {
        path,
        before_blob: blob_of(sink, old),
        after_blob: blob_of(sink, new),
        details,
        ..base(ObservationKind::FileEdited, tool_call, at)
    }
}

/// Observations implied by one tool call. `result` is Claude Code's structured `toolUseResult`.
pub fn derive_observations(name: &str, input: &Value, result: Option<&Value>, tool_call: ToolCallHandle, at: &Timestamp, sink: &mut dyn ParseSink) -> Vec<ObservationDraft> {
    let r = |k: &str| result.and_then(|r| str_field(r, k));
    match name {
        "Bash" => vec![ObservationDraft {
            command: str_field(input, "command"),
            details: json!({
                "description": str_field(input, "description"),
                "interrupted": result.and_then(|r| r.get("interrupted")).cloned().unwrap_or(Value::Null),
                "stderr_present": result.and_then(|r| r.get("stderr")).and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false),
            }),
            ..base(ObservationKind::CommandExecuted, tool_call, at)
        }],
        "Read" | "NotebookRead" => vec![ObservationDraft {
            path: str_field(input, "file_path").or_else(|| str_field(input, "notebook_path")),
            ..base(ObservationKind::FileRead, tool_call, at)
        }],
        "Glob" | "Grep" => vec![ObservationDraft {
            path: str_field(input, "path"),
            confidence: Confidence::Medium,
            details: json!({ "pattern": str_field(input, "pattern"), "tool": name }),
            ..base(ObservationKind::FileRead, tool_call, at)
        }],
        "Write" => vec![ObservationDraft {
            path: str_field(input, "file_path").or_else(|| r("filePath")),
            before_blob: blob_of(sink, r("originalFile")),
            after_blob: blob_of(sink, str_field(input, "content").or_else(|| r("content"))),
            details: json!({ "result_type": r("type"), "userModified": result.and_then(|x| x.get("userModified")).cloned().unwrap_or(Value::Null) }),
            ..base(ObservationKind::FileWritten, tool_call, at)
        }],
        "Edit" => vec![edit_observation(
            str_field(input, "file_path").or_else(|| r("filePath")),
            r("oldString").or_else(|| str_field(input, "old_string")),
            r("newString").or_else(|| str_field(input, "new_string")),
            json!({
                "replaceAll": result.and_then(|x| x.get("replaceAll")).cloned().unwrap_or(json!(false)),
                "structuredPatch": result.and_then(|x| x.get("structuredPatch")).cloned().unwrap_or(json!([])),
                "userModified": result.and_then(|x| x.get("userModified")).cloned().unwrap_or(Value::Null),
            }),
            tool_call,
            at,
            sink,
        )],
        "MultiEdit" => {
            let path = str_field(input, "file_path");
            input
                .get("edits")
                .and_then(Value::as_array)
                .map(|edits| {
                    edits
                        .iter()
                        .enumerate()
                        .map(|(i, e)| edit_observation(path.clone(), str_field(e, "old_string"), str_field(e, "new_string"), json!({ "edit_index": i, "replaceAll": e.get("replace_all").cloned().unwrap_or(json!(false)) }), tool_call, at, sink))
                        .collect()
                })
                .unwrap_or_default()
        }
        "NotebookEdit" => vec![edit_observation(
            str_field(input, "notebook_path"),
            None,
            str_field(input, "new_source"),
            json!({ "cell_id": str_field(input, "cell_id"), "edit_mode": str_field(input, "edit_mode") }),
            tool_call,
            at,
            sink,
        )],
        "WebFetch" => vec![ObservationDraft {
            path: str_field(input, "url"),
            details: json!({ "prompt": str_field(input, "prompt") }),
            ..base(ObservationKind::UrlReferenced, tool_call, at)
        }],
        "WebSearch" => vec![ObservationDraft {
            details: json!({ "query": str_field(input, "query") }),
            ..base(ObservationKind::UrlReferenced, tool_call, at)
        }],
        "Agent" | "Task" => vec![ObservationDraft {
            details: json!({
                "subagent_type": str_field(input, "subagent_type"),
                "description": str_field(input, "description"),
                "prompt_length": str_field(input, "prompt").map(|p| p.len()).unwrap_or(0),
                "agent_id": r("agentId"),
                "status": r("status"),
            }),
            ..base(ObservationKind::SubagentSpawned, tool_call, at)
        }],
        _ => Vec::new(),
    }
}

/// Registers `tool_use` blocks as pending and closes them when their `tool_result` arrives.
pub(crate) fn pair_blocks(state: &mut TranscriptState<'_>, message: MessageHandle, blocks: &[BlockDraft], tool_use_result: Option<&Value>, at: &Timestamp, prov: &Provenance, sink: &mut dyn ParseSink) {
    for (ordinal, b) in blocks.iter().enumerate() {
        let ordinal = ordinal as u32;
        match b.kind {
            BlockKind::ToolUse => {
                if let Some(id) = &b.tool_use_id {
                    state.pending.insert(
                        id.clone(),
                        PendingToolUse {
                            message,
                            ordinal,
                            name: str_field(&b.payload, "name").unwrap_or_default(),
                            input: b.payload.get("input").cloned().unwrap_or(Value::Null),
                            started: at.clone(),
                        },
                    );
                }
            }
            BlockKind::ToolResult => {
                let Some(id) = &b.tool_use_id else { continue };
                match state.pending.remove(id) {
                    Some(p) => {
                        let is_error = b.payload.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let handle = sink.tool_call(
                            state.session,
                            ToolCallDraft {
                                name: p.name.clone(),
                                category: categorize(&p.name),
                                input: p.input.clone(),
                                tool_use: BlockRef { message: p.message, ordinal: p.ordinal },
                                tool_result: Some(BlockRef { message, ordinal }),
                                result_text: b.text.clone(),
                                result_payload: tool_use_result.cloned(),
                                is_error,
                                started: p.started.clone(),
                                ended: at.clone(),
                            },
                        );
                        for obs in derive_observations(&p.name, &p.input, tool_use_result, handle, at, sink) {
                            sink.observation(state.session, obs);
                        }
                    }
                    None => sink.anomaly(AnomalyDraft {
                        kind: AnomalyKind::UnpairedToolResult,
                        severity: Severity::Warning,
                        source_file: Some(state.handle),
                        session: Some(state.session),
                        byte_offset: Some(prov.byte_offset),
                        message: format!("tool_result {id} has no tool_use in this file"),
                        details: json!({ "tool_use_id": id }),
                    }),
                }
            }
            _ => {}
        }
    }
}

/// Tool uses that never received a result (interrupted session) become result-less tool calls.
pub(crate) fn flush_unfinished(state: &mut TranscriptState<'_>, sink: &mut dyn ParseSink) {
    let mut pending: Vec<(String, PendingToolUse)> = state.pending.drain().collect();
    pending.sort_by_key(|a| (a.1.message.0, a.1.ordinal));
    for (_, p) in pending {
        let handle = sink.tool_call(
            state.session,
            ToolCallDraft {
                name: p.name.clone(),
                category: categorize(&p.name),
                input: p.input.clone(),
                tool_use: BlockRef { message: p.message, ordinal: p.ordinal },
                tool_result: None,
                result_text: None,
                result_payload: None,
                is_error: false,
                started: p.started.clone(),
                ended: Timestamp::absent(),
            },
        );
        for obs in derive_observations(&p.name, &p.input, None, handle, &p.started, sink) {
            sink.observation(state.session, obs);
        }
    }
}
