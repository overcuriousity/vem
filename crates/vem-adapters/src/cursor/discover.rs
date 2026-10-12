//! Identification by content signature and store discovery for `.cursor` (`cursor`) and Cursor's IDE
//! application directory (`cursor-ide`). Symbolic links are never followed; SQLite files are opened
//! only through [`open_immutable`].

use super::sqlite::{has_table, open_immutable};
use super::*;
use crate::claude_code::discover::list_files;
use std::io::Read;
use std::path::{Path, PathBuf};
use vem_core::adapter::{Discovery, Identification, StoreCandidate};
use vem_core::model::Harness;

const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

fn is_real_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.is_file())
        .unwrap_or(false)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

/// Immediate subdirectories of `dir` (not links), sorted by name.
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_real_dir(p))
        .collect();
    out.sort();
    out
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

fn is_sqlite(path: &Path) -> bool {
    if !is_real_file(path) {
        return false;
    }
    let mut head = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && &head == SQLITE_MAGIC
}

fn db_has_tables(path: &Path, tables: &[&str]) -> bool {
    is_sqlite(path)
        && open_immutable(path)
            .map(|c| tables.iter().all(|t| has_table(&c, t)))
            .unwrap_or(false)
}

/// A workspace `state.vscdb` whose `ItemTable` holds Cursor's `composer.composerData` key.
fn has_composer_data(path: &Path) -> bool {
    if !is_sqlite(path) {
        return false;
    }
    let Ok(c) = open_immutable(path) else {
        return false;
    };
    has_table(&c, "ItemTable")
        && c.query_row(
            "SELECT count(*) FROM ItemTable WHERE key = 'composer.composerData'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// `projects/<slug>/agent-transcripts` directories, sorted.
fn agent_transcript_dirs(root: &Path) -> Vec<PathBuf> {
    subdirs(&root.join("projects"))
        .into_iter()
        .map(|p| p.join("agent-transcripts"))
        .filter(|p| is_real_dir(p))
        .collect()
}

fn first_chat_store(root: &Path) -> Option<PathBuf> {
    subdirs(&root.join("chats"))
        .iter()
        .flat_map(|ws| subdirs(ws))
        .map(|d| d.join("store.db"))
        .find(|db| db_has_tables(db, &["blobs", "meta"]))
}

pub fn identify_cursor(root: &Path) -> Option<Identification> {
    let mut evidence = Vec::new();
    if let Some(d) = agent_transcript_dirs(root).first() {
        evidence.push(format!("agent-transcripts directory: {}", rel(root, d)));
    }
    if let Some(db) = first_chat_store(root) {
        evidence.push(format!(
            "chat store with tables blobs and meta: {}",
            rel(root, &db)
        ));
    }
    if is_real_dir(&root.join("acp-sessions")) {
        evidence.push("acp-sessions directory".to_string());
    }
    if is_real_file(&root.join("ai-tracking/ai-code-tracking.db")) {
        evidence.push("ai-tracking/ai-code-tracking.db".to_string());
    }
    if is_real_file(&root.join("prompt_history.json")) {
        evidence.push("prompt_history.json".to_string());
    }
    (!evidence.is_empty()).then_some(Identification {
        harness: Harness::Cursor,
        evidence,
    })
}

/// `User/workspaceStorage/` and `User/History/` alone do not identify: VS Code has them too.
pub fn identify_ide(root: &Path) -> Option<Identification> {
    let global = root.join("User/globalStorage/state.vscdb");
    let evidence = if db_has_tables(&global, &["cursorDiskKV"]) {
        "User/globalStorage/state.vscdb with table cursorDiskKV".to_string()
    } else {
        let ws = subdirs(&root.join("User/workspaceStorage"))
            .into_iter()
            .map(|d| d.join("state.vscdb"))
            .find(|db| has_composer_data(db))?;
        format!(
            "{} with ItemTable key composer.composerData",
            rel(root, &ws)
        )
    };
    Some(Identification {
        harness: Harness::CursorIde,
        evidence: vec![evidence],
    })
}

fn sort_slash(files: &mut [PathBuf]) {
    files.sort_by_cached_key(|p| {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    });
}

fn plain_store(d: &mut Discovery, root: &Path, kind: &str, rel: &str, generation: Option<&str>) {
    let rel_path = PathBuf::from(rel);
    // `symlink_metadata`: a store path that is a link is present (and manifested as a link), not followed.
    if std::fs::symlink_metadata(root.join(&rel_path)).is_err() {
        d.absent.push(kind.to_string());
        return;
    }
    d.stores.push(StoreCandidate {
        kind: kind.to_string(),
        generation: generation.map(str::to_string),
        files: list_files(root, &rel_path),
        rel_path,
    });
}

/// Stores of a `.cursor` directory, in parse order.
pub fn discover_cursor(root: &Path) -> Discovery {
    let mut d = Discovery::default();
    let dirs = agent_transcript_dirs(root);
    if dirs.is_empty() {
        d.absent.push(STORE_AGENT_TRANSCRIPTS.to_string());
    } else {
        let mut files: Vec<PathBuf> = dirs
            .iter()
            .flat_map(|a| list_files(root, a.strip_prefix(root).unwrap_or(a)))
            .collect();
        sort_slash(&mut files);
        let jsonl = files
            .iter()
            .any(|f| f.extension().is_some_and(|e| e == "jsonl"));
        d.stores.push(StoreCandidate {
            kind: STORE_AGENT_TRANSCRIPTS.to_string(),
            generation: Some(if jsonl { "jsonl" } else { "txt" }.to_string()),
            rel_path: PathBuf::from("projects"),
            files,
        });
    }
    plain_store(&mut d, root, STORE_CHAT_STORE, "chats", Some("store-db"));
    plain_store(&mut d, root, STORE_ACP_SESSIONS, "acp-sessions", None);
    plain_store(&mut d, root, STORE_AI_TRACKING, "ai-tracking", None);
    plain_store(
        &mut d,
        root,
        STORE_PROMPT_HISTORY,
        "prompt_history.json",
        None,
    );
    d
}

/// Stores of Cursor's IDE application directory, in parse order (workspace storage needs the composer
/// sessions of the global store).
pub fn discover_ide(root: &Path) -> Discovery {
    let mut d = Discovery::default();
    let global = Path::new("User/globalStorage");
    let db = global.join("state.vscdb");
    if is_real_file(&root.join(&db)) {
        let files: Vec<PathBuf> = ["state.vscdb", "state.vscdb-wal", "state.vscdb-shm"]
            .iter()
            .map(|n| global.join(n))
            .filter(|p| is_real_file(&root.join(p)))
            .collect();
        let generation = if db_has_tables(&root.join(&db), &["cursorDiskKV"]) {
            "cursor-disk-kv"
        } else {
            "itemtable-legacy"
        };
        d.stores.push(StoreCandidate {
            kind: STORE_STATE_VSCDB.to_string(),
            generation: Some(generation.to_string()),
            rel_path: global.to_path_buf(),
            files,
        });
    } else {
        d.absent.push(STORE_STATE_VSCDB.to_string());
    }
    plain_store(
        &mut d,
        root,
        STORE_WORKSPACE_STORAGE,
        "User/workspaceStorage",
        None,
    );
    plain_store(&mut d, root, STORE_LOCAL_HISTORY, "User/History", None);
    d
}
