//! Identification by content and store discovery for `.cursor` (`cursor`) and Cursor's IDE application
//! directory (`cursor-ide`).

use std::path::{Path, PathBuf};
use vem_adapters::cursor::discover::{discover_cursor, discover_ide};
use vem_adapters::identify_root;
use vem_core::model::Harness;

const T1: &str = "1c1c1c1c-0000-4000-8000-000000000001";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .canonicalize()
        .unwrap()
}

fn copy_dir(src: &Path, dst: &Path) {
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry.unwrap();
        let target = dst.join(entry.path().strip_prefix(src).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn identifies_dotcursor() {
    let ids = identify_root(&fixture("cursor/dotcursor"));
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert_eq!(ids[0].harness, Harness::Cursor);
    assert!(
        ids[0]
            .evidence
            .iter()
            .any(|e| e.contains("agent-transcripts")),
        "{:?}",
        ids[0].evidence
    );
}

#[test]
fn identifies_ide() {
    let ids = identify_root(&fixture("cursor/ide"));
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert_eq!(ids[0].harness, Harness::CursorIde);
    assert!(
        ids[0].evidence.iter().any(|e| e.contains("cursorDiskKV")),
        "{:?}",
        ids[0].evidence
    );
}

#[test]
fn ide_identified_by_workspace_composer_data_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Cursor");
    copy_dir(&fixture("cursor/ide"), &root);
    std::fs::remove_dir_all(root.join("User/globalStorage")).unwrap();
    let ids = identify_root(&root);
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert_eq!(ids[0].harness, Harness::CursorIde);
}

#[test]
fn vscode_dir_is_not_cursor_ide() {
    assert!(identify_root(&fixture("cursor/vscode")).is_empty());
}

#[test]
fn renamed_dotcursor_still_identified() {
    let tmp = tempfile::tempdir().unwrap();
    let renamed = tmp.path().join("evidence-42");
    copy_dir(&fixture("cursor/dotcursor"), &renamed);
    let ids = identify_root(&renamed);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].harness, Harness::Cursor);
}

#[test]
fn claude_fixture_not_cursor() {
    let ids = identify_root(&fixture("claude-code/basic"));
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert_eq!(ids[0].harness, Harness::ClaudeCode);
}

#[test]
fn non_sqlite_store_db_does_not_identify() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().join("chats/ws/x");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("store.db"), b"not a database at all").unwrap();
    assert!(identify_root(tmp.path()).is_empty());
}

#[test]
fn discovers_cursor_stores_in_order() {
    let d = discover_cursor(&fixture("cursor/dotcursor"));
    let kinds: Vec<&str> = d.stores.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "cursor:agent-transcripts",
            "cursor:chat-store",
            "cursor:acp-sessions",
            "cursor:prompt-history"
        ]
    );
    assert_eq!(d.absent, vec!["cursor:ai-tracking"]);

    let at = &d.stores[0];
    assert_eq!(at.rel_path, PathBuf::from("projects"));
    assert_eq!(at.generation.as_deref(), Some("jsonl"));
    let base = "projects/home-alice-proj/agent-transcripts";
    let want: Vec<PathBuf> = vec![
        format!("{base}/1c1c1c1c-0000-4000-8000-000000000000.txt"),
        format!("{base}/{T1}/{T1}.jsonl"),
        format!("{base}/{T1}/agent-tools/toolu_cursor_01.txt"),
        format!("{base}/{T1}/subagents/1c1c1c1c-0000-4000-8000-0000000000a1.jsonl"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    assert_eq!(at.files, want);

    let cs = &d.stores[1];
    assert_eq!(cs.rel_path, PathBuf::from("chats"));
    assert_eq!(cs.generation.as_deref(), Some("store-db"));
    assert_eq!(
        cs.files,
        vec![PathBuf::from(format!("chats/ws1/{T1}/store.db"))]
    );
    assert!(d.stores[2..].iter().all(|s| s.generation.is_none()));
    assert_eq!(
        d.stores[3].files,
        vec![PathBuf::from("prompt_history.json")]
    );
}

#[test]
fn transcripts_outside_agent_transcripts_are_not_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("c");
    copy_dir(&fixture("cursor/dotcursor"), &root);
    std::fs::write(root.join("projects/home-alice-proj/notes.txt"), "x").unwrap();
    let d = discover_cursor(&root);
    assert!(d.stores[0]
        .files
        .iter()
        .all(|f| f.components().any(|c| c.as_os_str() == "agent-transcripts")));
}

#[test]
fn txt_only_transcripts_are_generation_txt() {
    let tmp = tempfile::tempdir().unwrap();
    let at = tmp.path().join("projects/p/agent-transcripts");
    std::fs::create_dir_all(&at).unwrap();
    std::fs::write(at.join("x.txt"), "user: hi\n").unwrap();
    let d = discover_cursor(tmp.path());
    assert_eq!(d.stores[0].generation.as_deref(), Some("txt"));
}

#[test]
fn discovers_ide_stores() {
    let d = discover_ide(&fixture("cursor/ide"));
    let kinds: Vec<&str> = d.stores.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "cursor:state-vscdb",
            "cursor:workspace-storage",
            "cursor:local-history"
        ]
    );
    assert!(d.absent.is_empty(), "{:?}", d.absent);
    let kv = &d.stores[0];
    assert_eq!(kv.rel_path, PathBuf::from("User/globalStorage"));
    assert_eq!(kv.generation.as_deref(), Some("cursor-disk-kv"));
    assert_eq!(
        kv.files,
        vec![
            PathBuf::from("User/globalStorage/state.vscdb"),
            PathBuf::from("User/globalStorage/state.vscdb-wal")
        ]
    );
    assert!(d.stores[1]
        .files
        .contains(&PathBuf::from("User/workspaceStorage/ws1/workspace.json")));
    assert_eq!(d.stores[2].files.len(), 3);
}
