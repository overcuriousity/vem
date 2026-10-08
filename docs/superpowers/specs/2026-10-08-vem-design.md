# Vestigia Ex Machina (vem) — Design Specification

Date: 2026-10-08
Status: approved for planning

## 1. Purpose

`vem` is a forensic application for static, post-incident analysis of the on-disk traces
left by agentic AI coding harnesses. An examiner receives a collected home directory or a
mounted filesystem image, attaches it to a case, and reconstructs what the agents did on
that machine: what the user asked, what the assistant said, which commands ran, which
files changed, what data was exposed, and when.

The primary interface is a **conversation viewer**: the examiner selects a session and
reads user and assistant messages in order, drilling into tool calls, raw records and
provenance as deeply as the data allows. A unified timeline export (Timesketch and
Vestigo compatible) is a by-product of the same data.

The first prototype covers three harnesses: **Claude Code**, **Codex CLI**, and
**Cursor** (agent transcripts, IDE chat store, and the newer SQLite/protobuf chat store).

### Success criteria

- An examiner can attach an evidence root, run one ingest, and browse every discovered
  session of the three harnesses in a web UI without leaving the tool to inspect raw files.
- Every displayed message, block and derived fact can be traced to a file hash, byte
  offset and record hash of the original evidence.
- Timestamps and cross-store identity joins are never silently guessed; their origin and
  confidence are visible.
- Parse failures, truncation, deletions and other integrity signals are inventoried, not
  logged away.
- Exports load cleanly into Timesketch and Vestigo.

### Explicit assumptions

- Implementation language is Rust (the repository ships a Cargo `.gitignore`), Apache 2.0.
- Evidence arrives as a readable directory tree (collected home directory, mounted image,
  extracted triage archive). The tool never parses raw disk images.
- Examined machines may be Linux, macOS or Windows; the examiner workstation runs the tool
  locally, single user, offline.
- The Claude Code adapter is grounded in real data available during development. Codex
  and Cursor adapters are built from public format knowledge with synthesized fixtures;
  sanitized real trees replace those fixtures when available.

## 2. Architecture

Cargo workspace with four crates and one frontend package.

| Unit | Responsibility | Depends on |
|---|---|---|
| `vem-core` | Canonical model types, provenance and anomaly types, timestamp origin, tolerant JSONL reader, SHA-256 helpers, adapter traits (`Discover`, `Parse`). | std, serde, sha2 |
| `vem-adapters` | One module per harness (`claude_code`, `codex`, `cursor`), sub-modules per store generation. Pure: files in, canonical records + anomalies out. No database knowledge. | `vem-core`, rusqlite (read-only, immutable), a schemaless protobuf wire decoder |
| `vem-case` | Case directory and SQLite database, migrations, ingest pipeline, blob retention, FTS5 index, observation derivation, exports (Timesketch JSONL/CSV, Vestigo Parquet), audit log. | `vem-core`, `vem-adapters`, rusqlite, arrow/parquet |
| `vem` | Binary. CLI subcommands and the Axum HTTP server with the embedded frontend. | `vem-case`, axum, rust-embed |
| `frontend/` | React 19 + TypeScript + Vite single-page app, built to static files embedded into the binary. | — |

Data flow: `evidence root → discovery → manifest (hash) → retain copy → adapter parse
(streaming) → canonical rows + provenance + anomalies → observation derivation → FTS
index → UI / export`.

CLI surface (prototype):

```
vem case new <dir> --name <name> [--examiner <name>]
vem evidence add <case> <path> --label <label> [--host H] [--user U] [--os linux|macos|windows]
vem ingest <case> [--root <id>] [--no-retain]
vem verify <case>
vem export <case> --format timesketch-jsonl|timesketch-csv|vestigo-parquet [--root ID] [--session ID] -o <file>
vem serve <case> [--port 8787]        # binds 127.0.0.1 only
vem inventory <case>                  # text dump of stores, files, anomalies
```

## 3. Evidence and discovery

**Evidence root.** Path plus label, host, user, OS hint, and a `root_kind` classified
on attach:

- `FilesystemRoot`: contains `home/`, `Users/`, or `root/`; every user home found below
  is scanned (including WSL-mounted homes such as `mnt/c/Users/<u>`).
- `HomeDir`: contains dotfiles or `Library/`/`AppData/` directly.
- `HarnessDir`: the path itself is a harness configuration directory (basename
  `.claude`, `.codex`, `.cursor`, `Cursor`, or signature files present).

**Discovery strategies**, run in order; each discovered store records which one found it:

1. `Convention`: OS-conventional paths relative to each home directory
   (`~/.claude`, `~/.codex`, `~/.cursor`, `~/.config/cursor`, `~/.config/Cursor`,
   `~/Library/Application Support/Cursor`, `AppData/Roaming/Cursor`).
2. `EnvHint`: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_CONFIG_HOME` parsed from shell
   startup files in that home (`.bashrc`, `.bash_profile`, `.zshrc`, `.zshenv`,
   `.profile`, `.config/fish/config.fish`, `.config/environment.d/*.conf`). Values that
   reference `$HOME` are resolved against the evidence home, not the examiner's.
   Windows registry hints are out of scope for the prototype.
3. `SignatureScan`: bounded-depth walk (skipping `node_modules`, `.git`, `target`,
   caches) that recognizes stores by content: a `.jsonl` whose first record has
   `"type":"session_meta"` (Codex), a `.jsonl` whose records carry `parentUuid` and
   `sessionId` (Claude Code), a directory named `agent-transcripts`, a `state.vscdb`
   with a `cursorDiskKV` table, a `store.db` under a `chats/` tree.
4. `Manual`: the examiner points at a store directly.

**Read-only guarantee.** Evidence files are opened read-only. SQLite files inside the
evidence are opened with `?immutable=1` so no `-wal`/`-shm` files are created. Nothing
under an evidence root is ever created, modified or deleted by the tool.

## 4. Canonical model

Thin core, typed derived layer, everything pointing back to bytes.

```
Case          id, name, examiner, created_at, notes
EvidenceRoot  id, case_id, path, label, host, user, os, root_kind, attached_at
Store         id, root_id, harness {ClaudeCode|Codex|Cursor}, kind (string, e.g.
              claude:projects, claude:file-history, codex:sessions, codex:archived,
              cursor:agent-transcripts, cursor:state-vscdb, cursor:chat-store,
              cursor:ai-tracking, cursor:local-history), generation, path,
              discovery_method, status {Parsed|Inventoried|Failed}
SourceFile    id, store_id, rel_path, size, sha256, mtime, ctime, atime?, retained_blob?,
              parse_status, record_count, anomaly_count, ingested_at, version
Session       id, store_id, harness_session_id, kind {Primary|Subagent|Resumed|Forked},
              parent_session_id?, title?, project_path?, git_branch?, harness_version?,
              models (list), first_ts, first_ts_origin, last_ts, last_ts_origin,
              message_count, tool_call_count, primary_source_file_id
Message       id, session_id, ordinal, role {User|Assistant|System|Tool|Meta},
              harness_record_type, harness_uuid?, parent_uuid?, timestamp?, ts_origin,
              model?, provenance_id, attributes (JSON map)
Block         id, message_id, ordinal, kind {Text|Thinking|ToolUse|ToolResult|Image|
              Attachment|Other}, text?, payload (JSON), tool_call_id?, provenance_id
ToolCall      id, session_id, tool_use_block_id, tool_result_block_id?, name, input (JSON),
              result_text?, result_payload?, is_error, started_ts?, ended_ts?, ts_origin,
              category {Shell|FileRead|FileWrite|FileEdit|Search|Web|Agent|Mcp|Other}
Observation   id, session_id, kind {CommandExecuted|FileRead|FileWritten|FileEdited|
              FileDeleted|SubagentSpawned|UrlReferenced|SecretCandidate|PasteDetected|
              UploadDetected}, derived_from_block_id?, derived_from_tool_call_id?,
              path?, command?, before_blob?, after_blob?, timestamp?, ts_origin,
              confidence {High|Medium|Low}, details (JSON)
IdentityClaim id, session_id, scheme (e.g. cursor:composerId, cursor:store-session,
              codex:session_id, codex:id, claude:sessionId), claimed_id,
              source_file_id, join_status {Matched|Unmatched|Ambiguous}
Anomaly       id, root_id, store_id?, source_file_id?, session_id?, kind {TruncatedLine|
              MalformedRecord|UnknownRecordType|UnknownStoreGeneration|MissingTimestamp|
              OrphanedFile|SupersededFile|ArchivedSession|UnlinkedSubagent|
              FolderDateClockMismatch|HashDrift|EmptyStore|OversizedRecord},
              severity {Info|Warning|Error}, byte_offset?, message, details (JSON)
Provenance    id, source_file_id, byte_offset, byte_length, record_index,
              content_sha256, parser_name, parser_version, origin {Stored|Derived|Inferred}
ContentBlob   sha256 (pk), size, bytes (content-addressed; retained files, snapshots,
              sidecars, protobuf blobs)
Annotation    id, case_id, target_type, target_id, kind {Tag|Bookmark|Note}, value,
              created_at
AuditLog      id, case_id, ts, action, target, details (JSON)   -- append-only
```

**Timestamp origin** (`ts_origin`) on every timestamp-bearing row:
`Stored` (the record carries a UTC timestamp), `StoredLocalClock` (carried but known to
be local time, e.g. Codex folder dates), `FileMtime`, `NeighborInterpolated`, `Absent`.

**Nothing is dropped.** Unknown record types become `Meta` messages holding their raw
JSON in `attributes`, plus an `UnknownRecordType` anomaly at `Info` severity.

## 5. Provenance and integrity

- Every Message, Block and Anomaly carries a Provenance row: source file, byte offset
  and length, record index, SHA-256 of the raw record bytes, parser name and version.
- Attaching evidence builds a manifest: SHA-256, size and filesystem timestamps for every
  file in every discovered store. The whole-root manifest is optional (`--manifest-all`).
- By default ingest retains a content-addressed copy of each store file in the case
  directory, so the case remains self-contained after the evidence is unmounted and the
  raw-bytes view keeps working. `--no-retain` disables this.
- `vem verify` re-hashes evidence and retained blobs and records `HashDrift` anomalies.
- Claude Code file-history snapshots, Cursor local-history entries, checkpoints and
  `agent-tools/*.txt` sidecars become ContentBlobs referenced by Observations so the
  file-operation views can show before/after diffs.
- Ingest is idempotent by file hash. Unchanged files are skipped. A changed file is flagged
  and ingested as a new `SourceFile.version`; earlier rows are kept.

## 6. Adapters

Each adapter implements `Discover` (root → store candidates) and, per generation,
`Parse` (source file → stream of canonical records, tool calls, identity claims,
anomalies). Parsers are streaming and bounded in memory: no file is read whole.

### 6.1 Claude Code (`~/.claude` or `$CLAUDE_CONFIG_DIR`)

Stores: `projects/<encoded-cwd>/<uuid>.jsonl` (primary), `projects/<cwd>/<uuid>/subagents/*.jsonl`,
`*.orphaned-*` and `*.superseded-*` siblings (→ `OrphanedFile`/`SupersededFile` anomalies,
still parsed), `history.jsonl` (prompt history), `file-history/` (snapshots keyed by session
and tracking path), `shell-snapshots/*.sh`, `paste-cache/`, `uploads/`, `todos/`, `plans/`,
`session-env/`, `sessions/`, `settings.json`, `backups/`, `debug/`.

Record mapping: `user` / `assistant` / `system` → Message with content blocks (`text`,
`thinking`, `tool_use`, `tool_result`, `image`); `tool_use` ↔ `tool_result` paired by
`tool_use_id`; the structured `toolUseResult` field feeds Observations; `attachment` →
`Attachment` block on a `Meta` message; `file-history-snapshot` / `file-history-delta` →
ContentBlobs and `FileEdited` observations; `summary`, `ai-title`, `last-prompt` → session
title; `mode`, `permission-mode`, `queue-operation`, `bridge-session` and friends → `Meta`
messages. `isSidechain`, the `subagents/` directory, `parentToolUseId` /
`sourceToolUseID` → Subagent sessions linked to the spawning ToolCall. Session fields:
`cwd`, `gitBranch`, `version`, per-message `model`, `requestId`. Timestamps are ISO UTC →
`Stored`.

Observations: `Bash` → `CommandExecuted`; `Read`/`Glob`/`Grep` → `FileRead`;
`Write`/`Edit`/`MultiEdit`/`NotebookEdit` → `FileWritten`/`FileEdited` with before/after
from `toolUseResult` or file-history; `WebFetch`/`WebSearch` → `UrlReferenced`;
`Agent`/`Task` → `SubagentSpawned`; paste-cache and uploads → `PasteDetected`/`UploadDetected`.

### 6.2 Codex CLI (`~/.codex` or `$CODEX_HOME`)

Stores: `sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`, `archived_sessions/` (same
layout → `ArchivedSession` anomaly, Info), `session_index.jsonl`, `history.jsonl`,
`logs/`, `config.toml`.

Record mapping: first line `session_meta` → Session (`id`, `timestamp`, `cwd`,
`originator`, `cli_version`, git info, `source.subagent.thread_spawn.parent_thread_id` →
parent session). Sessions are keyed on `session_meta.id`; `session_id` is recorded only as
an IdentityClaim because it is reused across parent/child threads. `response_item`
(`message`, `function_call`, `function_call_output`, `local_shell_call`,
`custom_tool_call`, `web_search_call`, `reasoning`) → Message/Block/ToolCall;
`event_msg` (`user_message`, `agent_message`, `token_count`, `turn_aborted`, …) → Message
or Meta; `turn_context` (cwd, model, approval policy, sandbox) → session attributes;
`compacted` → Meta, with payload deduplicated by content hash. Per-line `timestamp` (UTC)
→ `Stored`. The folder date is compared with the session timestamp converted to local
time; disagreement → `FolderDateClockMismatch` (Info).

Observations: shell / `local_shell_call` / `exec_command` → `CommandExecuted`;
`apply_patch` → `FileEdited` per file in the patch (hunks retained as details);
`web_search_call` → `UrlReferenced`.

### 6.3 Cursor (`~/.cursor`, `$XDG_CONFIG_HOME/cursor`, IDE app-support directory)

Three generations, each a separate store kind with its own parser:

- **Agent transcripts** `projects/<slug>/agent-transcripts/<id>/<id>.jsonl` with
  `subagents/*.jsonl` and `agent-tools/*.txt`. Anthropic-style `role` + `message.content[]`
  → Message/Block directly. No per-record timestamps: session bounds come from file
  times (`FileMtime`), message timestamps are `Absent`. Sidecars become ContentBlobs.
- **IDE key-value store** `User/globalStorage/state.vscdb`, table `cursorDiskKV`:
  `composerData:<id>` → Session, `bubbleId:<composerId>:<bubbleId>` → Message (type 1
  user, 2 assistant; `createdAt` → `Stored`). `workspaceStorage/<hash>/state.vscdb`
  provides the composer-to-workspace link (project path).
- **Chat store** `chats/<workspace-hash>/<session-id>/store.db` (and `acp-sessions/`):
  **experimental**. Hex-encoded JSON metadata → Session fields; protobuf blobs are
  retained raw as ContentBlobs and walked with a schemaless wire-format decoder to
  extract strings, producing Messages with `origin = Derived`, `confidence = Low`.

Cross-generation ids (`composerId`, store.db session id, transcript id) are recorded as
IdentityClaims with join status; sessions are never merged on an unmatched claim.
`ai-tracking/ai-code-tracking.db` → attribution details on `FileEdited` observations;
`User/History/` → before/after ContentBlobs; `prompt_history.json` → `PasteDetected`-style
prompt observations.

## 7. Case database

One directory per case:

```
<case>/
  case.db          SQLite (WAL), schema versioned by migrations
  blobs/<sha256>   content-addressed retained files and snapshots
  exports/         hashed export files
```

Tables mirror §4. FTS5 virtual tables index message text, tool inputs and tool results.
Annotations and the audit log live in the same database. The audit log records evidence
attachment, ingest runs (with tool and parser versions), verify runs, annotations and
exports.

## 8. Conversation interface

Axum serves a JSON API and the embedded frontend on `127.0.0.1` only.

- **Case home**: evidence roots, discovered stores with discovery method, ingest status,
  inventory tree with hashes, anomaly list filterable by kind and severity.
- **Sessions list**: filter by harness, root, project, time range, kind, has subagents,
  has anomalies. Columns: harness, title or first prompt, project, start/end with
  origin badge, message and tool-call counts, models.
- **Session view (core)**: conversation stream of user and assistant messages in order.
  Tool calls render as one-line chips between messages (`Bash · git status · ok`) that
  expand to input and result. Thinking blocks collapsed. Meta and system records hidden
  behind a toggle. Header shows session metadata and a subagent tree for navigation
  to child and parent sessions. Each message shows a provenance badge: stored (green),
  inferred (amber), absent (grey). Selecting a message, block or tool call opens a
  detail drawer with: raw record JSON, raw bytes at the provenance offset, the
  Provenance row, derived Observations, IdentityClaims, Annotations.
- **Activity views**: commands table, file operations table with before/after diff
  viewer, indicators table (URLs, secret candidates, pastes, uploads). Every row links
  to its originating message.
- **Search**: case-wide FTS, results link to session and message.
- **Export**: choose format and scope, download; the export is hashed and audited.

## 9. Exports

**Timesketch JSONL / CSV**: one event per Message, ToolCall and Observation.
`message` is a compact rendering; `datetime` is the timestamp or empty; `timestamp_desc`
names the origin (e.g. `Message Timestamp (stored)`, `Session File Modified (inferred)`);
`source` is `AI:CLAUDE_CODE` / `AI:CODEX` / `AI:CURSOR`; `source_long` is
`<harness>:<record kind>`; `display_name` is root label plus relative path; `tags` carry
harness, role, origin and observation kind; attributes carry session id, message uuid,
model, tool name, cwd, file path, command, confidence.

**Vestigo Parquet** (interchange format v1): same events with `source_file` (evidence
relative path), `file_hash`, `byte_offset`, `content_hash` taken from Provenance rows.

Scope: whole case, one root, or one session.

## 10. Error handling

- Adapters never abort an ingest. A bad line → `MalformedRecord`/`TruncatedLine`
  anomaly with byte offset; parsing continues.
- A file that fails as a whole → `SourceFile.parse_status = Failed` with the error; the
  run continues with the next file.
- Records above a configurable size cap (default 64 MiB) are skipped with
  `OversizedRecord`.
- Stores of an unrecognized generation are inventoried (`status = Inventoried`) and
  hashed but not parsed, so the examiner sees they exist.
- Invalid UTF-8 is decoded lossily for display; hashes and offsets are computed on the
  original bytes.

## 11. Testing

- `fixtures/<harness>/<generation>/`: sanitized or synthesized evidence trees, including
  deliberately truncated files, orphaned and superseded siblings, archived Codex rollouts,
  a Cursor transcript tree without timestamps, and a `state.vscdb` with composer and
  bubble records.
- Adapter snapshot tests (`insta`) over canonical output for each fixture.
- Property tests for the tolerant JSONL reader: truncation at any byte, missing trailing
  newline, invalid UTF-8, CRLF, empty lines.
- Golden tests validating Timesketch JSONL/CSV field names and Vestigo Parquet schema
  against the documented specs.
- End-to-end test: create case, attach fixtures, ingest, query API for sessions,
  messages, observations and anomalies, export, verify.
- Read-only test: ingest a fixture under a watcher and assert no file under the root
  was created, modified or deleted.

## 12. Out of scope for the prototype

Raw disk image parsing; live acquisition on the examined host; Windows registry
environment hints; adapters for Gemini CLI, Copilot CLI, OpenCode, Aider, Kiro and Zed
(the adapter trait is designed to admit them); a generated written report; multi-user
authentication; statistical or embedding-based anomaly detection (Vestigo's domain);
a complete Cursor protobuf schema.
