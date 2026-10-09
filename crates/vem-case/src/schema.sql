-- vem case database, schema version 1. Mirrors spec §4 and §7.

CREATE TABLE case_info (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL,
    examiner TEXT,
    created_at TEXT NOT NULL,
    notes TEXT,
    tool_version TEXT NOT NULL
);

CREATE TABLE evidence_roots (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    label TEXT NOT NULL,
    host TEXT,
    user TEXT,
    os TEXT,
    harness TEXT NOT NULL,
    attached_at TEXT NOT NULL,
    identification TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE stores (
    id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES evidence_roots(id),
    harness TEXT NOT NULL,
    kind TEXT NOT NULL,
    generation TEXT,
    rel_path TEXT NOT NULL,
    discovery_method TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
);

CREATE TABLE absent_stores (
    id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES evidence_roots(id),
    kind TEXT NOT NULL
);

-- rel_path: '/'-separated path relative to the root. A name that is not valid UTF-8 is stored with each
-- invalid byte and each '%' percent-encoded (%XX) and rel_path_encoded = 1; decode with
-- evidence::decode_rel_path. kind: 'file', or 'symlink' (never followed; sha256 is the SHA-256 of the
-- link target string, size 0). ctime is the inode change time, btime the birth time where available.
CREATE TABLE source_files (
    id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES evidence_roots(id),
    store_id INTEGER REFERENCES stores(id),
    rel_path TEXT NOT NULL,
    rel_path_encoded INTEGER NOT NULL DEFAULT 0,
    kind TEXT NOT NULL DEFAULT 'file',
    link_target TEXT,
    size INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    mtime TEXT,
    ctime TEXT,
    btime TEXT,
    atime TEXT,
    retained INTEGER NOT NULL DEFAULT 0,
    parse_status TEXT NOT NULL DEFAULT 'unparsed',
    parse_error TEXT,
    record_count INTEGER NOT NULL DEFAULT 0,
    anomaly_count INTEGER NOT NULL DEFAULT 0,
    ingested_at TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    UNIQUE (root_id, rel_path, rel_path_encoded, version)
);

CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    store_id INTEGER NOT NULL REFERENCES stores(id),
    harness_session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    parent_session_id INTEGER REFERENCES sessions(id),
    parent_harness_session_id TEXT,
    title TEXT,
    project_path TEXT,
    git_branch TEXT,
    harness_version TEXT,
    models TEXT NOT NULL DEFAULT '[]',
    first_ts TEXT,
    first_ts_origin TEXT NOT NULL DEFAULT 'absent',
    last_ts TEXT,
    last_ts_origin TEXT NOT NULL DEFAULT 'absent',
    bounds_from_adapter INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    tool_call_count INTEGER NOT NULL DEFAULT 0,
    primary_source_file_id INTEGER REFERENCES source_files(id)
);
CREATE INDEX sessions_harness_id ON sessions(harness_session_id);
CREATE INDEX sessions_parent ON sessions(parent_session_id);

CREATE TABLE provenance (
    id INTEGER PRIMARY KEY,
    source_file_id INTEGER NOT NULL REFERENCES source_files(id),
    byte_offset INTEGER NOT NULL,
    byte_length INTEGER NOT NULL,
    record_index INTEGER NOT NULL,
    content_sha256 TEXT NOT NULL,
    parser_name TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    origin TEXT NOT NULL
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    ordinal INTEGER NOT NULL,
    role TEXT NOT NULL,
    harness_record_type TEXT NOT NULL,
    harness_uuid TEXT,
    parent_uuid TEXT,
    timestamp TEXT,
    ts_origin TEXT NOT NULL,
    model TEXT,
    provenance_id INTEGER NOT NULL REFERENCES provenance(id),
    attributes TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX messages_session ON messages(session_id, ordinal);

CREATE TABLE tool_calls (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    tool_use_block_id INTEGER NOT NULL,
    tool_result_block_id INTEGER,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    input TEXT NOT NULL,
    result_text TEXT,
    result_payload TEXT,
    is_error INTEGER NOT NULL DEFAULT 0,
    started_ts TEXT,
    ended_ts TEXT,
    ts_origin TEXT NOT NULL
);
CREATE INDEX tool_calls_session ON tool_calls(session_id);

CREATE TABLE blocks (
    id INTEGER PRIMARY KEY,
    message_id INTEGER NOT NULL REFERENCES messages(id),
    ordinal INTEGER NOT NULL,
    kind TEXT NOT NULL,
    text TEXT,
    payload TEXT NOT NULL DEFAULT 'null',
    tool_use_id TEXT,
    tool_call_id INTEGER REFERENCES tool_calls(id),
    provenance_id INTEGER NOT NULL REFERENCES provenance(id)
);
CREATE INDEX blocks_message ON blocks(message_id, ordinal);
CREATE INDEX blocks_tool_use_id ON blocks(tool_use_id);

CREATE TABLE observations (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    kind TEXT NOT NULL,
    derived_from_tool_call_id INTEGER REFERENCES tool_calls(id),
    derived_from_block_id INTEGER REFERENCES blocks(id),
    derived_from_provenance_id INTEGER REFERENCES provenance(id),
    path TEXT,
    command TEXT,
    before_blob TEXT,
    after_blob TEXT,
    timestamp TEXT,
    ts_origin TEXT NOT NULL,
    confidence TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX observations_session ON observations(session_id);
CREATE INDEX observations_kind ON observations(kind);

CREATE TABLE identity_claims (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    scheme TEXT NOT NULL,
    claimed_id TEXT NOT NULL,
    source_file_id INTEGER NOT NULL REFERENCES source_files(id),
    join_status TEXT NOT NULL,
    matched_session_id INTEGER REFERENCES sessions(id)
);

CREATE TABLE anomalies (
    id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES evidence_roots(id),
    store_id INTEGER REFERENCES stores(id),
    source_file_id INTEGER REFERENCES source_files(id),
    session_id INTEGER REFERENCES sessions(id),
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    byte_offset INTEGER,
    provenance_id INTEGER REFERENCES provenance(id),
    message TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX anomalies_root ON anomalies(root_id, kind);

CREATE TABLE blobs (
    sha256 TEXT PRIMARY KEY,
    size INTEGER NOT NULL
);

CREATE TABLE annotations (
    id INTEGER PRIMARY KEY,
    target_type TEXT NOT NULL,
    target_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY,
    ts TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT,
    details TEXT NOT NULL DEFAULT '{}'
);
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;

CREATE VIRTUAL TABLE blocks_fts USING fts5(text, content='blocks', content_rowid='id');
CREATE TRIGGER blocks_fts_ai AFTER INSERT ON blocks
BEGIN INSERT INTO blocks_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TRIGGER blocks_fts_ad AFTER DELETE ON blocks
BEGIN INSERT INTO blocks_fts(blocks_fts, rowid, text) VALUES ('delete', old.id, old.text); END;
CREATE TRIGGER blocks_fts_au AFTER UPDATE OF text ON blocks
BEGIN
    INSERT INTO blocks_fts(blocks_fts, rowid, text) VALUES ('delete', old.id, old.text);
    INSERT INTO blocks_fts(rowid, text) VALUES (new.id, new.text);
END;

-- Tool inputs and results are searchable too (spec §7): name, input JSON and result text.
CREATE VIRTUAL TABLE tool_calls_fts USING fts5(name, input, result_text, content='tool_calls', content_rowid='id');
CREATE TRIGGER tool_calls_fts_ai AFTER INSERT ON tool_calls
BEGIN INSERT INTO tool_calls_fts(rowid, name, input, result_text) VALUES (new.id, new.name, new.input, new.result_text); END;
CREATE TRIGGER tool_calls_fts_ad AFTER DELETE ON tool_calls
BEGIN INSERT INTO tool_calls_fts(tool_calls_fts, rowid, name, input, result_text) VALUES ('delete', old.id, old.name, old.input, old.result_text); END;
CREATE TRIGGER tool_calls_fts_au AFTER UPDATE OF name, input, result_text ON tool_calls
BEGIN
    INSERT INTO tool_calls_fts(tool_calls_fts, rowid, name, input, result_text) VALUES ('delete', old.id, old.name, old.input, old.result_text);
    INSERT INTO tool_calls_fts(rowid, name, input, result_text) VALUES (new.id, new.name, new.input, new.result_text);
END;
