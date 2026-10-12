//! Cursor's tool vocabulary (both the agent-transcript and the IDE names) and the observations derived
//! from a tool call.

use crate::claude_code::transcript::str_field;
use serde_json::{json, Value};
use vem_core::model::*;

/// `name` without a trailing `_v<digits>` (`run_terminal_cmd_v2` is `run_terminal_cmd`).
fn base_name(name: &str) -> &str {
    match name.rsplit_once("_v") {
        Some((head, digits))
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        }
        _ => name,
    }
}

fn is_delete(name: &str) -> bool {
    matches!(base_name(name), "Delete" | "delete_file")
}

pub fn categorize(name: &str) -> ToolCategory {
    match base_name(name) {
        "Shell" | "run_terminal_cmd" | "run_terminal_command" => ToolCategory::Shell,
        "Read" | "read_file" | "ReadLints" => ToolCategory::FileRead,
        "Write" | "write" | "create_file" => ToolCategory::FileWrite,
        "StrReplace" | "search_replace" | "edit_file" | "MultiEdit" | "EditNotebook"
        | "apply_patch" | "Delete" | "delete_file" => ToolCategory::FileEdit,
        "Glob" | "Grep" | "LS" | "list_dir" | "grep_search" | "file_search" | "codebase_search"
        | "SemanticSearch" | "glob_file_search" => ToolCategory::Search,
        "WebSearch" | "WebFetch" | "web_search" | "fetch_url" => ToolCategory::Web,
        "Task" => ToolCategory::Agent,
        n if n.starts_with("mcp_") => ToolCategory::Mcp,
        _ => ToolCategory::Other,
    }
}

fn path_of(input: &Value) -> Option<String> {
    [
        "path",
        "file_path",
        "target_file",
        "relative_workspace_path",
    ]
    .iter()
    .find_map(|k| str_field(input, k))
}

/// Observations implied by one Cursor tool call. `result_ok` is whether the call is known to have
/// completed; only then is the observation `High` confidence.
pub fn derive_observations(
    name: &str,
    input: &Value,
    result_ok: Option<bool>,
    tc: ToolCallHandle,
    at: &Timestamp,
) -> Vec<ObservationDraft> {
    let base = |kind| ObservationDraft {
        kind,
        derived_from: Derivation::ToolCall(tc),
        path: None,
        command: None,
        before_blob: None,
        after_blob: None,
        timestamp: at.clone(),
        confidence: if result_ok == Some(true) {
            Confidence::High
        } else {
            Confidence::Medium
        },
        details: json!({ "tool": name }),
    };
    let obs = match categorize(name) {
        ToolCategory::Shell => ObservationDraft {
            command: str_field(input, "command"),
            ..base(ObservationKind::CommandExecuted)
        },
        ToolCategory::FileRead => ObservationDraft {
            path: path_of(input),
            ..base(ObservationKind::FileRead)
        },
        ToolCategory::FileWrite => ObservationDraft {
            path: path_of(input),
            ..base(ObservationKind::FileWritten)
        },
        ToolCategory::FileEdit => ObservationDraft {
            path: path_of(input),
            ..base(if is_delete(name) {
                ObservationKind::FileDeleted
            } else {
                ObservationKind::FileEdited
            })
        },
        ToolCategory::Web => ObservationDraft {
            path: str_field(input, "url").or_else(|| str_field(input, "query")),
            ..base(ObservationKind::UrlReferenced)
        },
        ToolCategory::Agent => base(ObservationKind::SubagentSpawned),
        ToolCategory::Search | ToolCategory::Mcp | ToolCategory::Other => return Vec::new(),
    };
    vec![obs]
}
