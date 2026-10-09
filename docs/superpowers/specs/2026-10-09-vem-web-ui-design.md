# vem plan 2 — Web API and Conversation UI: Design

Date: 2026-10-09
Status: approved in brainstorming, pending written-spec review
Parent spec: `2026-10-08-vem-design.md` (this document refines its §8 and the §4, §6.1 and §7 items deferred by plan 1)

## 1. Goal and scope

`vem serve <case> [--port 8787]` serves a JSON API over an existing case and an embedded single-page
conversation viewer, bound to `127.0.0.1` only. The release artifact is **one self-contained binary**
that can be copied to an air-gapped examiner workstation: no Node, no network, no external assets at
runtime.

The UI is **read-only over the case** except for two audited writes: annotations and exports. Creating
cases, attaching evidence, ingest and verify stay on the CLI.

Also in scope: `paste-cache` and `uploads` derivations, secret-candidate scanning, the annotations write
path, and four findings parked by the plan 1 review.

Out of scope: Codex (plan 3), Cursor (plan 4), triggering ingest/verify/attach from the UI, multi-user
access, authentication beyond the loopback bind.

## 2. Settled decisions

| Question | Decision |
|---|---|
| Frontend build wiring | `frontend/dist` is gitignored. `build.rs` never runs npm. Debug/test builds tolerate a missing `dist` (a "frontend not built" page is served; the API works). A `--release` build fails if `frontend/dist/index.html` is missing. `scripts/build-release.sh` runs `npm ci`, `npm run build`, `cargo build --release`. |
| UI scope | Read-only plus annotations and export. |
| Routing and state | React Router + TanStack Query. All view state (session, selection, filters, meta toggle) lives in the URL. No global store. |
| Diff | Server-side line diff (`similar` crate) of two retained blobs; the API returns hunks; React renders unified and split views. Binary content is detected and not diffed. |
| Raw bytes | Served base64 with a `verified` flag after re-hashing against `content_sha256`; rendered as hex + ASCII with file-relative offsets and a lossy-UTF-8 text toggle; paged in 64 KiB windows. |
| Server crate | New library crate `crates/vem-web` (router, handlers, assets), called by `vem serve`. Updates parent spec §2. |

## 3. Architecture

```
vem (bin)  ──serve──▶  vem-web (lib: axum Router, assets, guards)
                            │
                            ▼
                       vem-case (query, annotations, export::run, diff, derivations)
                            │
                            ▼
                     vem-core / vem-adapters
frontend/ (Vite build) ──embedded by rust-embed──▶ vem-web
```

### 3.1 `vem-case` additions

- `query.rs`: `audit_log()`; `SessionFilter` gains `from`, `to` (compared against `first_ts`/`last_ts`),
  `has_children`, `has_anomalies`; `AnomalyFilter` gains `session_id`; `ObservationFilter` gains
  `kinds: Vec<String>`; `message(id)` (single message with blocks); `message_for_block(id)` and
  `message_for_tool_call(id)`; `ingest_status(root_id)` (file counts per `parse_status`, last ingest audit
  entry); `search` hits enriched with session title and message ordinal; `blob(sha) -> Option<Vec<u8>>`
  with a size cap.
- `annotations.rs`: `create(case, target_type, target_id, kind, value)`, `list(case, target_type?,
  target_id?)`, `delete(case, id)`. `target_type` ∈ {session, message, block, tool_call, observation,
  anomaly, source_file} and must reference an existing row; `kind` ∈ {tag, bookmark, note}. Each create and
  delete writes an audit entry whose details hold the full annotation, so the annotation history can be
  rebuilt from the audit log alone.
- `export/mod.rs`: `run(case, format, scope, out_path) -> ExportReport { format, output, events, sha256 }`,
  moved from `crates/vem/src/commands.rs`; the CLI and server both call it; it checks scope and output
  path, refuses an empty scope, hashes, audits.
- `diff.rs`: `diff_bytes(old: &[u8], new: &[u8]) -> DiffResult { binary: bool,
  hunks: Vec<Hunk { old_start, old_lines, new_start, new_lines, lines: Vec<Line { tag: Equal|Insert|Delete,
  old_no, new_no, text }> }> }`; the `/api/diff` route reads the two blobs with `query::blob_bytes`
  and calls it. An absent side is empty (creation or deletion). Content is binary when it
  contains a NUL byte in its first 8 KiB or is not valid UTF-8. Context: 3 lines.
- `secrets.rs`: the secret-candidate rule set (§5.3).

### 3.2 `vem-web`

- `serve(case_dir, port)`: refuses a non-case directory; binds `SocketAddr::from((Ipv4Addr::LOCALHOST,
  port))` (no host option exists); prints `http://127.0.0.1:<port>/`.
- State: `case_dir` plus one `Mutex<Case>` for short queries. Long work (export, diff, raw bytes, blob
  read) opens its own `Case` connection. All database and file work runs in `spawn_blocking`.
- Guards (middleware, every request):
  - `Host` must equal `127.0.0.1:<port>` or `localhost:<port>`, else 421. Defeats DNS rebinding.
  - Non-GET requests require `Content-Type: application/json` and, when an `Origin` header is present,
    an `Origin` equal to `http://127.0.0.1:<port>` or `http://localhost:<port>`, else 403. Defeats
    cross-site writes from other local pages.
  - Response headers: `Content-Security-Policy: default-src 'self'; img-src 'self' data:; style-src
    'self' 'unsafe-inline'`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
    `Cache-Control: no-store` on `/api`.
  - Handler panics become 500 (`tower_http::catch_panic`).
- Assets: rust-embed over `frontend/dist` with `allow_missing`; `/assets/*` served with their MIME type;
  any other non-`/api` GET serves `index.html` (client routing).
- No outbound network code; no HTTP client dependency.

### 3.3 Frontend (`frontend/`)

Vite + React 19 + TypeScript (strict) + React Router + TanStack Query. Plain CSS with design tokens,
light and dark following the OS; all fonts and icons bundled (system font stack + bundled monospace
fallback; no web fonts fetched). `src/api/types.ts` mirrors the Rust response structs;
`src/api/client.ts` wraps `fetch` with typed functions. Vitest + Testing Library.

## 4. API

All responses JSON. Timestamps pass through as stored with their `ts_origin`. Errors: `{ "error":
string, "kind": string }` with 404 (unknown id), 409 (integrity mismatch), 422 (bad input), 500.

| Method, path | Returns |
|---|---|
| `GET /api/case` | case info + totals (sessions, messages, tool_calls, observations, anomalies by severity) |
| `GET /api/roots` | roots, each with stores, absent sub-stores, ingest status |
| `GET /api/roots/{id}/files` | source files (inventory) |
| `GET /api/anomalies?root&kind&severity&session` | anomalies |
| `GET /api/audit` | audit log |
| `GET /api/sessions?harness&root&kind&project&from&to&has_children&has_anomalies` | sessions |
| `GET /api/sessions/{id}` | session + `ancestors` (root-first chain) + `children` + `claims` |
| `GET /api/sessions/{id}/messages?meta=true\|false` | messages with blocks |
| `GET /api/sessions/{id}/tool-calls` | tool calls |
| `GET /api/sessions/{id}/observations` | observations |
| `GET /api/messages/{id}` | message + blocks + observations derived from it, its blocks or its tool calls + annotations on all of those |
| `GET /api/provenance/{id}` | provenance row |
| `GET /api/provenance/{id}/raw?offset&len` | `{ total_length, offset, bytes_b64, verified }`; the whole record is re-hashed before any window is served; mismatch → 409; `len` capped at 64 KiB |
| `GET /api/activity/commands?root&session` | `command_executed` observations + session/message link |
| `GET /api/activity/files?root&session` | `file_read/written/edited/deleted` observations + link |
| `GET /api/activity/indicators?root&session&kind` | `url_referenced`, `secret_candidate`, `paste_detected`, `upload_detected` + link |
| `GET /api/blobs/{sha}` | `{ size, binary, text? , bytes_b64? }`, capped at 4 MiB with `truncated` |
| `GET /api/diff?before&after` | `DiffResult` |
| `GET /api/search?q&limit` | hits with session title and message ordinal |
| `GET /api/annotations?target_type&target_id` | annotations |
| `POST /api/annotations` | created annotation |
| `DELETE /api/annotations/{id}` | 204 |
| `POST /api/exports {format, scope: {case} \| {root: id} \| {session: id}}` | `ExportReport` with `name` |
| `GET /api/exports` | earlier exports (from the audit log, with existence flag) |
| `GET /api/exports/{name}` | file download; `name` must equal an entry of `<case>/exports/` (no path components) |

Activity rows carry `session_id` and `message_id` (resolved through the derived block or tool call's
`tool_use_block_id`; null when the observation derives from a sidecar record without a message).

Exports from the UI are written to `<case>/exports/<UTC yyyymmddThhmmssZ>-<scope>.<jsonl|csv|parquet>`.

## 5. Ingest-side additions

### 5.1 `ParseSink::read_root_file`

`fn read_root_file(&self, rel_path: &Path) -> Result<Option<Vec<u8>>, String>`: looks the path up in the
root's manifest (latest version). `None` if absent from the manifest or a symlink. Reads the retained blob
when `retained`, otherwise the live evidence file; re-hashes against the manifest `sha256`; a mismatch is
`Err` (the caller records `hash_drift`, Error) and the bytes are not used. A size cap of 64 MiB applies.
Test sinks implement it from an in-memory map.

### 5.2 Paste-cache and uploads

- A `history.jsonl` `pastedContents` entry with `contentHash` (no inline `content`) refers to
  `paste-cache/<contentHash>.txt`. The history parser reads it through `read_root_file`, stores it as a
  blob, and the existing `paste_detected` observation gains `details.pastes: [{ id, content_hash,
  content_blob?, size?, inline: bool, missing: bool }]`. A referenced file that is absent sets
  `missing: true` (deletion evidence; the observation stays).
- After ingest of a root, every `claude:paste-cache` source file whose stem no `paste_detected`
  observation references gets an `orphaned_file` anomaly (Info): pasted content with no referencing
  prompt; prompt history may have been cleared.
- `uploads/<sessionId>/<name>` (store `claude:uploads`): one `upload_detected` observation per file on
  session `<sessionId>`, `path` = file name, `details { content_blob, size, mime }` (MIME sniffed from
  magic bytes: png, jpeg, gif, webp, pdf, else by extension, else `application/octet-stream`),
  confidence High, timestamp = file mtime (`file_mtime` origin). Unknown session id: as for history, a
  `sidecar_only` session plus `missing_transcript`; if the transcript is in the manifest but unparsed,
  the file fails and is retried.
- `claude:uploads` becomes a parsed store kind. `claude:paste-cache` stays inventoried: its files are
  read through `history.jsonl` references.

### 5.3 Secret candidates

Derived in `vem-case` as `DbSink` inserts block text, tool-call input (serialized) and result text,
and paste content (inline in `history.jsonl` or from `paste-cache/`). Rules, each with a stable id:

| id | pattern (summary) | confidence |
|---|---|---|
| `aws-access-key-id` | `\b(AKIA\|ASIA)[0-9A-Z]{16}\b` | high |
| `github-token` | `\bgh[pousr]_[A-Za-z0-9]{36,}\b`, `\bgithub_pat_[A-Za-z0-9_]{22,}\b` | high |
| `gitlab-token` | `\bglpat-[A-Za-z0-9_-]{20,}\b` | high |
| `slack-token` | `\bxox[abposr]-[A-Za-z0-9-]{10,}\b` | high |
| `anthropic-key` | `\bsk-ant-[A-Za-z0-9_-]{20,}\b` | high |
| `openai-key` | `\bsk-(proj-)?[A-Za-z0-9_-]{20,}\b` (not `sk-ant-`) | high |
| `google-api-key` | `\bAIza[0-9A-Za-z_-]{35}\b` | high |
| `private-key` | `-----BEGIN [A-Z ]*PRIVATE KEY-----` | high |
| `jwt` | `\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}` | medium |
| `generic-assignment` | `(?i)(password\|passwd\|secret\|token\|api[_-]?key)\s*[:=]\s*["']?[^\s"']{8,}` | low |

Each match yields one `secret_candidate` observation linked to its block or tool call (paste matches link
to the `paste_detected` record's provenance), `details { rule, rule_version, field, offset, length,
match }`. The ingest audit entry records the rule-set version. The UI masks `match` by default with a
reveal toggle. Duplicate matches of the same rule at the same offset of the same source are emitted once.

### 5.4 Parked fixes

1. **atime before discovery.** `attach` first walks the root (no following links) collecting
   `symlink_metadata` for every entry, then identifies and discovers, then hashes; the manifest uses the
   snapshot. Test: atime recorded equals the atime before attach.
2. **Backups from retained copies.** `file_history_delta` reads the backup through `read_root_file`.
   Test: attach with retention, delete the evidence copy (a temp copy of the fixture), ingest; the
   `file_edited` observation still has its `before_blob`.
3. **Attach survives unreadable files.** A file that cannot be opened, copied or hashed is recorded as
   an `unreadable_file` anomaly (Error, `details { rel_path, error }`, no source file), listed in
   `report.unreadable`, and attach continues. New `AnomalyKind::UnreadableFile`; parent spec §4 gains it.
   Test: a chmod-000 file in a temp tree (skipped when running as root).
4. **Orphan blobs.** `DbSink` records blob files it created (did not exist before). When the file's
   transaction is rolled back, ingest removes those files that no `blobs` row references. `verify`
   reports blob files present on disk but absent from the index (`unindexed_blobs`), read-only.

### 5.5 Fixture

New `fixtures/claude-code/exposure/` (synthesized): one transcript session with an `Edit` and a `Bash`
call whose input contains a fake `ghp_` token; a `history.jsonl` line with a `contentHash` paste and one
with an inline paste containing `AKIAIOSFODNN7EXAMPLE`; `paste-cache/<hash>.txt` for the referenced paste
plus one unreferenced paste-cache file; `uploads/<known session>/…-shot.png` and `uploads/<unknown
session>/…-doc.pdf`; a file-history backup; a block with a PEM private-key header and a
`password=hunter2hunter2` line. `fixtures/claude-code/basic` is not modified; its counts (5 sessions,
23 messages, 4 tool calls, 6 observations, 4 anomalies, 33 events) change only by secret candidates
derived from its content (the inline `AKIAIOSFODNN7EXAMPLE` paste), and tests that pin those counts are
updated deliberately with the reason in the commit message.

Existing cases ingested by plan 1 do not gain the new derivations; the README says to ingest into a new
case.

## 6. Pages

App shell: top bar with case name, examiner, global search box, nav (Home · Sessions · Activity ·
Search · Export · Audit).

**Origin language (everywhere):** `stored` green solid badge; `inferred`/`derived` amber dashed badge
(`neighbor_interpolated`, `file_mtime`, derived provenance); `absent` grey hollow badge;
`stored_local_clock` blue badge. Tooltip names the exact origin. Derived content (observations,
`derived`-provenance messages) has an amber left rule.

- **Case home** (`/`): root cards (label, harness, host/user/os, path, identification evidence); store
  table (kind, generation, discovery method, status, files); absent sub-stores callout phrased as what to
  request; ingest status counts with failed files and errors; collapsible inventory tree (size, sha256
  prefix with copy, parse status, version, retained, symlink target); anomaly table filterable by kind,
  severity, root, each row linking to its session/message or raw bytes.
- **Sessions** (`/sessions`): filter bar (harness, root, kind, project, from/to, has subagents, has
  anomalies); table (harness, title or first prompt, project, start/end with badges, messages, tool calls,
  models, anomalies); subagents indented under their parent.
- **Session view** (`/sessions/:id?meta=1&sel=message:12`): header (title, project, branch, harness
  version, models, start–end with badges, anomaly count), subagent tree (ancestors, this, children),
  meta toggle. Stream in ordinal order; each message shows role, timestamp + badge, model. Thinking
  collapsed with its length. Tool calls are one-line chips `name · key argument · ok|error|no result`
  (key argument: command, path, pattern, URL, or subagent description), expandable to input and result;
  file chips link to the diff; Agent/Task chips link to the child session. With meta hidden a divider
  "N meta records hidden" marks each run of hidden records. Unknown blocks render as raw JSON. Clicking
  a message, block or chip sets `sel` and opens the drawer. Keys: j/k next/previous message, Enter open
  drawer, Esc close.
- **Detail drawer** tabs: Record JSON (parsed from the verified raw bytes, pretty); Raw bytes (hex +
  ASCII, file-relative offsets, text toggle, verification result, paging); Provenance (file path, file
  sha256, offset, length, record index, parser name/version, origin); Observations; Identity claims;
  Annotations (list, add tag/bookmark/note, delete).
- **Activity** (`/activity/:tab`): Commands; File operations with a diff panel (unified/split, link to the
  originating message); Indicators (URLs, secret candidates masked with reveal, pastes with content
  preview, uploads with image preview for image MIME types served as blob data URLs). Every row links to
  its originating message.
- **Search** (`/search?q=`): hits grouped by session with highlighted snippets; links to
  `/sessions/:id?sel=message:N`, which scrolls to and flashes the message.
- **Export** (`/export`): format + scope pickers, run, table of earlier exports with sha256 and download.
- **Audit** (`/audit`): read-only audit log table.

## 7. Error handling

`CaseError` → status: not found 404; `IntegrityMismatch` 409; bad filter, bad annotation target, bad
export scope 422; else 500 with the message. The UI shows integrity failures prominently in the drawer
instead of hiding the record. `vem serve` exits non-zero on a non-case path or a bind failure.

## 8. Testing

- Rust in-process API tests (`tower::ServiceExt::oneshot`) over cases ingested from `basic` and
  `exposure`: counts, session tree, messages with and without meta, raw bytes verified, raw-bytes 409 on
  a tampered retained blob, diff hunks for the fixture edit, search enrichment, activity links,
  annotations create/list/delete with audit entries, export run + list + download, and the audit log
  never shrinking.
- Security tests: wrong `Host` → 421; cross-origin POST → 403; non-JSON POST → 403; export name with `/`
  or `..` → 404; CSP header present on HTML and API responses.
- Derivation tests: paste-cache reference resolved, missing paste flagged, unreferenced paste anomaly,
  upload on known and unknown session, each secret rule positive and one negative per rule.
- One regression test per parked fix (§5.4).
- Read-only test extended to the new fixture: no file under the root is created, modified or deleted.
- Vitest: chip rendering, collapsed thinking, meta divider, hex view offsets, diff render, origin badges.
- Gates: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and for waves
  touching `frontend/`, `npm run build` and `npm test`.
