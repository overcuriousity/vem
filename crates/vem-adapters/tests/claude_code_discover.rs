use std::path::{Path, PathBuf};
use vem_adapters::claude_code::{
    ClaudeCodeAdapter, STORE_FILE_HISTORY, STORE_HISTORY, STORE_PROJECTS,
};
use vem_core::adapter::HarnessAdapter;
use vem_core::model::Harness;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code/basic")
        .canonicalize()
        .unwrap()
}

#[test]
fn identifies_fixture_as_claude_code_by_content() {
    let id = ClaudeCodeAdapter.identify(&fixture()).expect("identified");
    assert_eq!(id.harness, Harness::ClaudeCode);
    assert!(
        id.evidence.iter().any(|e| e.contains("parentUuid")),
        "{:?}",
        id.evidence
    );
}

#[test]
fn identification_ignores_folder_name() {
    let tmp = tempfile::tempdir().unwrap();
    let renamed = tmp.path().join("evidence_item_7");
    copy_dir(&fixture(), &renamed);
    assert!(ClaudeCodeAdapter.identify(&renamed).is_some());
}

#[test]
fn unrelated_directory_is_not_identified() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("projects")).unwrap();
    std::fs::write(tmp.path().join("settings.json"), "{}").unwrap();
    assert!(ClaudeCodeAdapter.identify(tmp.path()).is_none());
}

#[test]
fn discovers_stores_and_reports_absent_ones() {
    let d = ClaudeCodeAdapter.discover(&fixture());
    let kinds: Vec<&str> = d.stores.iter().map(|s| s.kind.as_str()).collect();
    assert_eq!(
        kinds[0], STORE_PROJECTS,
        "projects store must come first so sidecars can find sessions"
    );
    assert!(kinds.contains(&STORE_HISTORY));
    assert!(kinds.contains(&STORE_FILE_HISTORY));
    assert!(kinds.contains(&"claude:shell-snapshots"));
    assert!(kinds.contains(&"claude:settings"));
    assert!(d.absent.contains(&"claude:todos".to_string()));
    assert!(d.absent.contains(&"claude:plans".to_string()));
    let projects = &d.stores[0];
    assert_eq!(projects.generation.as_deref(), Some("2.1.294"));
    let files: Vec<String> = projects
        .files
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(files.contains(
        &"projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl".to_string()
    ));
    assert!(files.contains(&"projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/subagents/agent-0123456789abcdef.jsonl".to_string()));
    assert!(files.contains(&"projects/-home-alice-proj/22222222-0000-4000-8000-000000000002.jsonl.orphaned-1759221000000".to_string()));
    assert!(
        files.windows(2).all(|w| w[0] <= w[1]),
        "files must be sorted"
    );
}

#[test]
fn registry_identifies_root() {
    let ids = vem_adapters::identify_root(&fixture());
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].harness, Harness::ClaudeCode);
    assert!(vem_adapters::adapter_for(Harness::ClaudeCode).is_some());
    assert!(vem_adapters::adapter_for(Harness::Codex).is_none());
}

fn copy_dir(src: &Path, dst: &Path) {
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(src).unwrap();
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}
