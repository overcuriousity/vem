# vem Foundation + Claude Code Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A headless `vem` CLI that creates a case, attaches a collected `.claude` directory, hashes and retains it, parses every Claude Code session into the canonical model with per-record provenance, derives command/file/subagent observations, and exports Timesketch JSONL/CSV and Vestigo Parquet.

**Architecture:** Cargo workspace of four crates. `vem-core` holds the canonical model, provenance types, the tolerant JSONL reader and the adapter/sink traits. `vem-adapters` holds the Claude Code adapter as pure functions from files to canonical drafts emitted into a `ParseSink`. `vem-case` owns the SQLite case database, the ingest pipeline (a `ParseSink` that writes rows), verification, queries and exports. `vem` is the clap CLI. The web UI (plan 2), Codex (plan 3) and Cursor (plan 4) build on these interfaces without changing them.

**Tech Stack:** Rust 1.98 (edition 2021), rusqlite 0.40 (bundled SQLite with FTS5), serde/serde_json, sha2, chrono, walkdir, clap 4, arrow/parquet 60, csv 1, thiserror 2; tests with insta (json), proptest, assert_cmd, tempfile.

**Spec:** `docs/superpowers/specs/2026-10-08-vem-design.md`

**Follow-on plans (not in this document):** plan 2 = Axum JSON API + embedded React conversation UI (`vem serve`); plan 3 = Codex adapter; plan 4 = Cursor adapters (agent transcripts, `state.vscdb`, `store.db`).

## Global Constraints

- Toolchain lives in `~/.cargo/bin`; every shell step begins with `export PATH="$HOME/.cargo/bin:$PATH"`.
- Workspace `rust-version = "1.85"`, `edition = "2021"`, license `Apache-2.0`. No `unsafe`.
- Nothing under an evidence root is ever created, modified or deleted (spec §3). Open evidence files read-only. Never open an evidence SQLite file without `?immutable=1` (not needed in this plan; Claude Code has no SQLite).
- Timestamps are stored as UTC strings in the exact format `%Y-%m-%dT%H:%M:%S%.3fZ` (e.g. `2026-09-30T10:00:00.000Z`) and every timestamp carries a `ts_origin` (spec §4).
- Every Message, Block and Anomaly carries a Provenance row: source file, byte offset, byte length, record index, SHA-256 of the raw record bytes, parser name, parser version, origin (spec §5).
- Nothing is dropped: unknown record types become `meta` messages with the raw JSON in `attributes` plus an `unknown_record_type` anomaly at `info` severity (spec §4).
- Adapters never abort an ingest on a bad record; a bad line becomes an anomaly and parsing continues (spec §10). Records above 64 MiB are skipped with `oversized_record`.
- Ingest is idempotent by file hash: unchanged files are skipped (spec §5).
- Vestigo Parquet export must match the version-1 schema exactly and carry footer metadata `vestigo.format_version="1"`, `vestigo.converter_name`, `vestigo.converter_version`, `vestigo.original_files` (JSON array of `{name, sha256, size_bytes, path, mtime}`) (spec §9, verified against Vestigo's `parquet_format.py`).
- The audit log is append-only (spec §7): enforced by triggers.
- Two anomaly kinds are added beyond the spec's list, and the spec's §4 list is to be updated by Task 4: `unpaired_tool_result` (a tool_result with no matching tool_use in the same file) and `missing_transcript` (a sidecar references a session that has no transcript).
- Commit after every task with a message in the form `feat(scope): ...`, `test(scope): ...` or `chore: ...`, ending with the line `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. A conversation record whose `message.content` is missing, an empty array, or a non-string/non-array value must yield a message with zero or one `other` block, never a panic. (Test added to Task 6.)
2. A `tool_result` whose `tool_use` lives in another file, or a `tool_use` that never gets a result because the session was interrupted, must produce an `unpaired_tool_result` anomaly or a result-less tool call respectively, never a lost record. (Tests added to Task 7.)
3. A `history.jsonl` line whose `timestamp` is a string or a float instead of integer milliseconds must still produce the observation, with an `absent` timestamp when unparseable. (Test added to Task 9.)
4. A timestamp with a non-zero offset such as `2026-09-30T12:00:00.000+02:00` must normalize to `2026-09-30T10:00:00.000Z`; a timestamp without offset or unparseable must be treated as absent with a `missing_timestamp` anomaly on conversation records. (Tests added to Task 4 and Task 6.)
5. Running `vem ingest` twice on the same case must not duplicate sessions, messages or anomalies. (Test added to Task 12.)

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `crates/vem-core/Cargo.toml`, `crates/vem-core/src/lib.rs`
- Create: `crates/vem-adapters/Cargo.toml`, `crates/vem-adapters/src/lib.rs`
- Create: `crates/vem-case/Cargo.toml`, `crates/vem-case/src/lib.rs`
- Create: `crates/vem/Cargo.toml`, `crates/vem/src/main.rs`
- Modify: `README.md`

**Interfaces:**
- Produces: the four crate names `vem-core`, `vem-adapters`, `vem-case`, `vem` and the shared `[workspace.dependencies]` every later task references.

- [ ] **Step 1: Write the workspace manifest**

`Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/vem-core", "crates/vem-adapters", "crates/vem-case", "crates/vem"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
rust-version = "1.85"
repository = "https://github.com/overcuriousity/vem"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }
sha2 = "0.10"
hex = "0.4"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
walkdir = "2"
rusqlite = { version = "0.40", features = ["bundled"] }
clap = { version = "4", features = ["derive"] }
arrow = "60"
parquet = "60"
csv = "1"
tempfile = "3"
insta = { version = "1", features = ["json"] }
proptest = "1"
assert_cmd = "2"
predicates = "3"
```

- [ ] **Step 2: Write the four crate manifests and stub sources**

`crates/vem-core/Cargo.toml`:

```toml
[package]
name = "vem-core"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Canonical model, provenance types and adapter traits for vem"

[dependencies]
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
hex.workspace = true
chrono.workspace = true
thiserror.workspace = true

[dev-dependencies]
proptest.workspace = true
tempfile.workspace = true
```

`crates/vem-core/src/lib.rs`:

```rust
//! Canonical model, provenance and adapter traits shared by every vem crate.
```

`crates/vem-adapters/Cargo.toml`:

```toml
[package]
name = "vem-adapters"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Harness adapters: files in, canonical records out"

[dependencies]
vem-core = { path = "../vem-core" }
serde_json.workspace = true
walkdir.workspace = true

[dev-dependencies]
insta.workspace = true
tempfile.workspace = true
```

`crates/vem-adapters/src/lib.rs`:

```rust
//! Harness adapters. Each adapter is a pure function from evidence files to canonical drafts.
```

`crates/vem-case/Cargo.toml`:

```toml
[package]
name = "vem-case"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Case database, ingest pipeline, verification and exports for vem"

[dependencies]
vem-core = { path = "../vem-core" }
vem-adapters = { path = "../vem-adapters" }
rusqlite.workspace = true
serde.workspace = true
serde_json.workspace = true
chrono.workspace = true
thiserror.workspace = true
walkdir.workspace = true
arrow.workspace = true
parquet.workspace = true
csv.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/vem-case/src/lib.rs`:

```rust
//! Case directory, SQLite database, ingest pipeline, verification, queries and exports.
```

`crates/vem/Cargo.toml`:

```toml
[package]
name = "vem"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Vestigia Ex Machina: forensic analysis of agentic AI harness traces"

[[bin]]
name = "vem"
path = "src/main.rs"

[dependencies]
vem-core = { path = "../vem-core" }
vem-case = { path = "../vem-case" }
clap.workspace = true
serde_json.workspace = true

[dev-dependencies]
assert_cmd.workspace = true
predicates.workspace = true
tempfile.workspace = true
```

`crates/vem/src/main.rs`:

```rust
fn main() {
    println!("vem {}", env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 3: Build and run the empty workspace**

Run:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd /home/user01/vem && cargo build --workspace && cargo run -p vem
```
Expected: build succeeds, output `vem 0.1.0`.

- [ ] **Step 4: Extend README**

Append to `README.md`:

```markdown

Forensic analysis of the on-disk traces left by agentic AI coding harnesses
(Claude Code, Codex CLI, Cursor). Attach a collected harness directory to a case,
ingest it, browse sessions with per-record provenance, export Timesketch and
Vestigo timelines.

Design: `docs/superpowers/specs/2026-10-08-vem-design.md`.

Build: `cargo build --workspace` (Rust 1.85+).
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates README.md
git commit -m "chore: scaffold cargo workspace with core, adapters, case and cli crates

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: SHA-256 helpers

**Files:**
- Create: `crates/vem-core/src/hash.rs`
- Modify: `crates/vem-core/src/lib.rs`

**Interfaces:**
- Produces: `vem_core::hash::sha256_hex(bytes: &[u8]) -> String` (lowercase hex) and `vem_core::hash::sha256_file(path: &Path) -> io::Result<(String, u64)>` returning `(hex_digest, byte_size)`.

- [ ] **Step 1: Write the failing tests**

`crates/vem-core/src/hash.rs`:

```rust
//! SHA-256 helpers. Every file in an evidence root and every parsed record is hashed with these.

use sha2::{Digest, Sha256};
use std::io::{self, Read};
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hashes_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hashes_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn file_hash_matches_bytes_hash_and_reports_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::File::create(&p).unwrap().write_all(&data).unwrap();
        let (digest, size) = sha256_file(&p).unwrap();
        assert_eq!(digest, sha256_hex(&data));
        assert_eq!(size, data.len() as u64);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod hash;` to `crates/vem-core/src/lib.rs`, then run:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p vem-core hash
```
Expected: compile error, `sha256_hex` and `sha256_file` not found.

- [ ] **Step 3: Implement**

Insert above the `#[cfg(test)]` block in `crates/vem-core/src/hash.rs`:

```rust
/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Streams `path` through SHA-256 in 64 KiB chunks. Returns `(hex_digest, size_in_bytes)`.
pub fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), total))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p vem-core hash`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/vem-core
git commit -m "feat(core): add sha256 helpers for bytes and files

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Tolerant JSONL reader

**Files:**
- Create: `crates/vem-core/src/jsonl.rs`
- Create: `crates/vem-core/tests/jsonl_props.rs`
- Modify: `crates/vem-core/src/lib.rs`

**Interfaces:**
- Produces: `vem_core::jsonl::RawRecord { index: u64, offset: u64, length: u64, bytes: Vec<u8>, terminated: bool, oversized: bool }` and `vem_core::jsonl::JsonlReader<R: BufRead>` with `new(inner)` (64 MiB cap), `with_max_len(inner, max_len)`, implementing `Iterator<Item = io::Result<RawRecord>>`. Blank lines are skipped but still counted in `index`. `bytes` never contains the trailing `\n` or `\r`. `length` is the on-disk line length excluding the newline and a trailing `\r`, even when `oversized` capped `bytes`.

- [ ] **Step 1: Write the failing unit tests**

`crates/vem-core/src/jsonl.rs`:

```rust
//! Tolerant JSONL reader: byte offsets for provenance, truncated last line, CRLF, blank lines,
//! and an upper bound on record size so a corrupt file cannot exhaust memory.

use std::io::{self, BufRead};

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read_all(data: &[u8], cap: usize) -> Vec<RawRecord> {
        JsonlReader::with_max_len(Cursor::new(data.to_vec()), cap)
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn yields_offsets_and_lengths_for_terminated_lines() {
        let recs = read_all(b"{\"a\":1}\n{\"b\":22}\n", 1024);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].index, 0);
        assert_eq!(recs[0].offset, 0);
        assert_eq!(recs[0].length, 7);
        assert_eq!(recs[0].bytes, b"{\"a\":1}");
        assert!(recs[0].terminated);
        assert_eq!(recs[1].index, 1);
        assert_eq!(recs[1].offset, 8);
        assert_eq!(recs[1].length, 8);
        assert!(recs[1].terminated);
    }

    #[test]
    fn last_line_without_newline_is_unterminated() {
        let recs = read_all(b"{\"a\":1}\n{\"b\":2", 1024);
        assert_eq!(recs.len(), 2);
        assert!(!recs[1].terminated);
        assert_eq!(recs[1].bytes, b"{\"b\":2");
        assert_eq!(recs[1].length, 6);
    }

    #[test]
    fn strips_carriage_return_and_counts_it_in_length() {
        let recs = read_all(b"{\"a\":1}\r\n{\"b\":2}\r\n", 1024);
        assert_eq!(recs[0].bytes, b"{\"a\":1}");
        assert_eq!(recs[0].length, 7);
        assert_eq!(recs[1].offset, 9);
    }

    #[test]
    fn skips_blank_lines_but_keeps_line_index() {
        let recs = read_all(b"{\"a\":1}\n\n\n{\"b\":2}\n", 1024);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1].index, 3);
        assert_eq!(recs[1].offset, 9);
    }

    #[test]
    fn caps_oversized_lines_and_keeps_subsequent_offsets_right() {
        let recs = read_all(b"abcdefgh\n{\"b\":2}\n", 4);
        assert_eq!(recs.len(), 2);
        assert!(recs[0].oversized);
        assert_eq!(recs[0].bytes, b"abcd");
        assert_eq!(recs[0].length, 8);
        assert!(!recs[1].oversized);
        assert_eq!(recs[1].offset, 9);
        assert_eq!(recs[1].bytes, b"{\"b\":2}");
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(read_all(b"", 1024).is_empty());
        assert!(read_all(b"\n\n", 1024).is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod jsonl;` to `crates/vem-core/src/lib.rs`, then run `cargo test -p vem-core jsonl`.
Expected: compile error, `RawRecord`/`JsonlReader` not found.

- [ ] **Step 3: Implement the reader**

Insert above the `#[cfg(test)]` block in `crates/vem-core/src/jsonl.rs`:

```rust
/// One physical line of a JSONL file, with the byte range it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    /// 0-based physical line number (blank lines count).
    pub index: u64,
    /// Byte offset of the first byte of the line.
    pub offset: u64,
    /// Length of the line on disk excluding the newline and a trailing `\r`.
    pub length: u64,
    /// Line content without newline or trailing `\r`; capped at `max_len` when `oversized`.
    pub bytes: Vec<u8>,
    /// `false` when the file ended without a newline after this line.
    pub terminated: bool,
    /// `true` when the line exceeded `max_len`; `bytes` holds only the first `max_len` bytes.
    pub oversized: bool,
}

pub const DEFAULT_MAX_LEN: usize = 64 * 1024 * 1024;

pub struct JsonlReader<R: BufRead> {
    inner: R,
    offset: u64,
    index: u64,
    max_len: usize,
    done: bool,
}

impl<R: BufRead> JsonlReader<R> {
    pub fn new(inner: R) -> Self {
        Self::with_max_len(inner, DEFAULT_MAX_LEN)
    }

    pub fn with_max_len(inner: R, max_len: usize) -> Self {
        Self { inner, offset: 0, index: 0, max_len, done: false }
    }

    /// Reads one physical line. Returns `Ok(None)` at EOF.
    fn read_line(&mut self) -> io::Result<Option<RawRecord>> {
        let start = self.offset;
        let mut bytes: Vec<u8> = Vec::new();
        let mut consumed: u64 = 0;
        let mut oversized = false;
        let mut terminated = false;
        let mut last_byte: Option<u8> = None;
        loop {
            let buf = self.inner.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            let (chunk, used, hit_newline) = match buf.iter().position(|&b| b == b'\n') {
                Some(pos) => (&buf[..pos], pos + 1, true),
                None => (buf, buf.len(), false),
            };
            if let Some(&b) = chunk.last() {
                last_byte = Some(b);
            }
            let room = self.max_len.saturating_sub(bytes.len());
            if chunk.len() > room {
                bytes.extend_from_slice(&chunk[..room]);
                oversized = true;
            } else {
                bytes.extend_from_slice(chunk);
            }
            consumed += used as u64;
            self.inner.consume(used);
            if hit_newline {
                terminated = true;
                break;
            }
        }
        if consumed == 0 {
            return Ok(None);
        }
        self.offset += consumed;
        let index = self.index;
        self.index += 1;
        let mut length = consumed - if terminated { 1 } else { 0 };
        if last_byte == Some(b'\r') {
            length = length.saturating_sub(1);
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        Ok(Some(RawRecord { index, offset: start, length, bytes, terminated, oversized }))
    }
}

impl<R: BufRead> Iterator for JsonlReader<R> {
    type Item = io::Result<RawRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.done {
                return None;
            }
            match self.read_line() {
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
                Ok(None) => {
                    self.done = true;
                    return None;
                }
                Ok(Some(rec)) => {
                    if !rec.terminated {
                        self.done = true;
                    }
                    if rec.length == 0 {
                        continue;
                    }
                    return Some(Ok(rec));
                }
            }
        }
    }
}
```

- [ ] **Step 4: Run the unit tests to verify they pass**

Run: `cargo test -p vem-core jsonl`
Expected: 6 passed.

- [ ] **Step 5: Write the property test**

`crates/vem-core/tests/jsonl_props.rs`:

```rust
use proptest::prelude::*;
use std::io::Cursor;
use vem_core::jsonl::JsonlReader;

fn line_strategy() -> impl Strategy<Value = Vec<u8>> {
    // Bytes that are not '\n' and not '\r', so each generated line is exactly one record.
    prop::collection::vec(any::<u8>().prop_filter("no newline", |b| *b != b'\n' && *b != b'\r'), 0..40)
}

proptest! {
    #[test]
    fn offsets_index_back_into_the_original_bytes(
        lines in prop::collection::vec(line_strategy(), 0..12),
        trailing_newline in any::<bool>(),
    ) {
        let mut data = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            data.extend_from_slice(l);
            if i + 1 < lines.len() || trailing_newline {
                data.push(b'\n');
            }
        }
        let recs: Vec<_> = JsonlReader::new(Cursor::new(data.clone())).map(|r| r.unwrap()).collect();
        let non_empty: Vec<_> = lines.iter().enumerate().filter(|(_, l)| !l.is_empty()).collect();
        prop_assert_eq!(recs.len(), non_empty.len());
        for (rec, (idx, line)) in recs.iter().zip(non_empty.iter()) {
            prop_assert_eq!(rec.index as usize, *idx);
            prop_assert_eq!(&rec.bytes, *line);
            let slice = &data[rec.offset as usize..(rec.offset + rec.length) as usize];
            prop_assert_eq!(slice, line.as_slice());
        }
        if let Some(last) = recs.last() {
            let last_line_is_final = non_empty.last().map(|(i, _)| *i + 1 == lines.len()).unwrap_or(false);
            prop_assert_eq!(last.terminated, !(last_line_is_final && !trailing_newline));
        }
    }
}
```

- [ ] **Step 6: Run the property test**

Run: `cargo test -p vem-core --test jsonl_props`
Expected: 1 passed.

- [ ] **Step 7: Commit**

```bash
git add crates/vem-core
git commit -m "feat(core): tolerant jsonl reader with byte offsets, truncation and size cap

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Canonical model, sink trait, adapter trait, test sink

**Files:**
- Create: `crates/vem-core/src/model.rs`
- Create: `crates/vem-core/src/sink.rs`
- Create: `crates/vem-core/src/adapter.rs`
- Create: `crates/vem-core/src/testing.rs`
- Modify: `crates/vem-core/src/lib.rs`
- Modify: `docs/superpowers/specs/2026-10-08-vem-design.md` (append two anomaly kinds to §4)

**Interfaces:**
- Produces (all `pub` in `vem_core::model`): string enums `Harness`, `TsOrigin`, `Role`, `BlockKind`, `ToolCategory`, `ObservationKind`, `Confidence`, `SessionKind`, `JoinStatus`, `AnomalyKind`, `Severity`, `ProvOrigin`, each with `as_str()`, `parse(&str) -> Option<Self>` and `Display`; `Timestamp { value: Option<String>, origin: TsOrigin }` with `absent()`, `stored(&str) -> Option<Self>`, `stored_epoch_ms(i64) -> Option<Self>`, `from_mtime(SystemTime)`; `TS_FORMAT`; handles `SourceFileHandle(i64)`, `SessionHandle(i64)`, `MessageHandle(i64)`, `ToolCallHandle(i64)`; `Provenance`; drafts `SessionDraft`, `SessionUpdate`, `MessageDraft`, `BlockDraft`, `BlockRef`, `ToolCallDraft`, `Derivation`, `ObservationDraft`, `IdentityClaimDraft`, `AnomalyDraft`.
- Produces `vem_core::sink::ParseSink` trait and `vem_core::adapter::{HarnessAdapter, Identification, StoreCandidate, Discovery, FileContext, ParseError}`.
- Produces `vem_core::testing::VecSink`, an in-memory `ParseSink` for adapter tests.

- [ ] **Step 1: Write the failing tests for timestamp normalization**

Create `crates/vem-core/src/model.rs` with only this test module for now:

```rust
//! Canonical model shared by adapters and the case database (spec §4).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::SystemTime;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_offsets_to_utc_millis() {
        assert_eq!(
            Timestamp::stored("2026-09-30T12:00:00.000+02:00").unwrap().value.as_deref(),
            Some("2026-09-30T10:00:00.000Z")
        );
        assert_eq!(
            Timestamp::stored("2026-09-30T10:00:00Z").unwrap().value.as_deref(),
            Some("2026-09-30T10:00:00.000Z")
        );
        assert_eq!(Timestamp::stored("2026-09-30T10:00:00.123456Z").unwrap().value.as_deref(), Some("2026-09-30T10:00:00.123Z"));
    }

    #[test]
    fn rejects_unparseable_or_offsetless_timestamps() {
        assert!(Timestamp::stored("yesterday").is_none());
        assert!(Timestamp::stored("2026-09-30 10:00:00").is_none());
        assert!(Timestamp::stored("").is_none());
    }

    #[test]
    fn epoch_millis_and_mtime() {
        assert_eq!(Timestamp::stored_epoch_ms(1_790_000_000_000).unwrap().value.as_deref(), Some("2026-09-21T14:13:20.000Z"));
        let t = Timestamp::from_mtime(SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_790_000_000_500));
        assert_eq!(t.origin, TsOrigin::FileMtime);
        assert_eq!(t.value.as_deref(), Some("2026-09-21T14:13:20.500Z"));
        assert_eq!(Timestamp::absent().origin, TsOrigin::Absent);
    }

    #[test]
    fn enums_round_trip_through_strings_and_serde() {
        assert_eq!(Harness::ClaudeCode.as_str(), "claude-code");
        assert_eq!(Harness::parse("cursor-ide"), Some(Harness::CursorIde));
        assert_eq!(AnomalyKind::parse("nope"), None);
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
        let k: ObservationKind = serde_json::from_str("\"file_edited\"").unwrap();
        assert_eq!(k, ObservationKind::FileEdited);
        assert_eq!(TsOrigin::StoredLocalClock.to_string(), "stored_local_clock");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Add to `crates/vem-core/src/lib.rs`:

```rust
pub mod adapter;
pub mod model;
pub mod sink;
pub mod testing;
```

Create empty `crates/vem-core/src/adapter.rs`, `sink.rs`, `testing.rs` (each with a one-line `//!` doc comment). Run `cargo test -p vem-core model`. Expected: compile errors for missing types.

- [ ] **Step 3: Implement the model**

Insert into `crates/vem-core/src/model.rs` between the `use` lines and the test module:

```rust
macro_rules! str_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $s:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $( #[serde(rename = $s)] $variant ),+ }
        impl $name {
            pub fn as_str(&self) -> &'static str { match self { $(Self::$variant => $s),+ } }
            pub fn parse(s: &str) -> Option<Self> { match s { $($s => Some(Self::$variant),)+ _ => None } }
            pub const ALL: &'static [$name] = &[$(Self::$variant),+];
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
        }
    };
}

str_enum!(/// Which harness produced a store. `CursorIde` is Cursor's IDE application directory.
    Harness { ClaudeCode = "claude-code", Codex = "codex", Cursor = "cursor", CursorIde = "cursor-ide" });
str_enum!(/// Where a timestamp came from (spec §4).
    TsOrigin { Stored = "stored", StoredLocalClock = "stored_local_clock", FileMtime = "file_mtime", NeighborInterpolated = "neighbor_interpolated", Absent = "absent" });
str_enum!(Role { User = "user", Assistant = "assistant", System = "system", Tool = "tool", Meta = "meta" });
str_enum!(BlockKind { Text = "text", Thinking = "thinking", ToolUse = "tool_use", ToolResult = "tool_result", Image = "image", Attachment = "attachment", Other = "other" });
str_enum!(ToolCategory { Shell = "shell", FileRead = "file_read", FileWrite = "file_write", FileEdit = "file_edit", Search = "search", Web = "web", Agent = "agent", Mcp = "mcp", Other = "other" });
str_enum!(ObservationKind { CommandExecuted = "command_executed", FileRead = "file_read", FileWritten = "file_written", FileEdited = "file_edited", FileDeleted = "file_deleted", SubagentSpawned = "subagent_spawned", UrlReferenced = "url_referenced", SecretCandidate = "secret_candidate", PasteDetected = "paste_detected", UploadDetected = "upload_detected" });
str_enum!(Confidence { High = "high", Medium = "medium", Low = "low" });
str_enum!(SessionKind { Primary = "primary", Subagent = "subagent", Resumed = "resumed", Forked = "forked" });
str_enum!(JoinStatus { Matched = "matched", Unmatched = "unmatched", Ambiguous = "ambiguous" });
str_enum!(AnomalyKind {
    TruncatedLine = "truncated_line", MalformedRecord = "malformed_record", UnknownRecordType = "unknown_record_type",
    UnknownStoreGeneration = "unknown_store_generation", MissingTimestamp = "missing_timestamp", OrphanedFile = "orphaned_file",
    SupersededFile = "superseded_file", ArchivedSession = "archived_session", UnlinkedSubagent = "unlinked_subagent",
    FolderDateClockMismatch = "folder_date_clock_mismatch", HashDrift = "hash_drift", EmptyStore = "empty_store",
    OversizedRecord = "oversized_record", UnpairedToolResult = "unpaired_tool_result", MissingTranscript = "missing_transcript",
});
str_enum!(Severity { Info = "info", Warning = "warning", Error = "error" });
str_enum!(ProvOrigin { Stored = "stored", Derived = "derived", Inferred = "inferred" });

/// The one timestamp format stored anywhere in vem: UTC, millisecond precision, `Z` suffix.
pub const TS_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

pub fn normalize_rfc3339(raw: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc).format(TS_FORMAT).to_string())
}

pub fn normalize_epoch_ms(ms: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms).map(|d| d.format(TS_FORMAT).to_string())
}

pub fn format_system_time(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t).format(TS_FORMAT).to_string()
}

/// A timestamp plus where it came from. `value` is always in `TS_FORMAT` when present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timestamp {
    pub value: Option<String>,
    pub origin: TsOrigin,
}

impl Timestamp {
    pub fn absent() -> Self { Self { value: None, origin: TsOrigin::Absent } }
    pub fn stored(raw: &str) -> Option<Self> {
        normalize_rfc3339(raw).map(|v| Self { value: Some(v), origin: TsOrigin::Stored })
    }
    pub fn stored_epoch_ms(ms: i64) -> Option<Self> {
        normalize_epoch_ms(ms).map(|v| Self { value: Some(v), origin: TsOrigin::Stored })
    }
    pub fn from_mtime(t: SystemTime) -> Self {
        Self { value: Some(format_system_time(t)), origin: TsOrigin::FileMtime }
    }
    pub fn is_present(&self) -> bool { self.value.is_some() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceFileHandle(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionHandle(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageHandle(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallHandle(pub i64);

/// Where a parsed row came from, down to the byte range (spec §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_file: SourceFileHandle,
    pub byte_offset: u64,
    pub byte_length: u64,
    pub record_index: u64,
    pub content_sha256: String,
    pub parser_name: String,
    pub parser_version: String,
    pub origin: ProvOrigin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDraft {
    pub harness_session_id: String,
    pub kind: SessionKind,
    pub parent_harness_session_id: Option<String>,
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub harness_version: Option<String>,
    /// When `None`, the case computes bounds from message timestamps, then file mtime.
    pub first_ts: Option<Timestamp>,
    pub last_ts: Option<Timestamp>,
}

/// Fields an adapter learns after it has already emitted the session. Only `Some` fields are applied,
/// and they never overwrite a value already set (first seen wins), except `title` which last-seen wins.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionUpdate {
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub harness_version: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockDraft {
    pub kind: BlockKind,
    pub text: Option<String>,
    pub payload: Value,
    pub tool_use_id: Option<String>,
}

impl BlockDraft {
    pub fn text(s: &str) -> Self {
        Self { kind: BlockKind::Text, text: Some(s.to_string()), payload: Value::Null, tool_use_id: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageDraft {
    pub harness_record_type: String,
    pub harness_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub role: Role,
    pub timestamp: Timestamp,
    pub model: Option<String>,
    pub attributes: serde_json::Map<String, Value>,
    pub blocks: Vec<BlockDraft>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRef {
    pub message: MessageHandle,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallDraft {
    pub name: String,
    pub category: ToolCategory,
    pub input: Value,
    pub tool_use: BlockRef,
    pub tool_result: Option<BlockRef>,
    pub result_text: Option<String>,
    pub result_payload: Option<Value>,
    pub is_error: bool,
    pub started: Timestamp,
    pub ended: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Derivation {
    ToolCall(ToolCallHandle),
    Block(BlockRef),
    Record(Provenance),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationDraft {
    pub kind: ObservationKind,
    pub derived_from: Derivation,
    pub path: Option<String>,
    pub command: Option<String>,
    pub before_blob: Option<String>,
    pub after_blob: Option<String>,
    pub timestamp: Timestamp,
    pub confidence: Confidence,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityClaimDraft {
    pub scheme: String,
    pub claimed_id: String,
    pub source_file: SourceFileHandle,
    pub join_status: JoinStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyDraft {
    pub kind: AnomalyKind,
    pub severity: Severity,
    pub source_file: Option<SourceFileHandle>,
    pub session: Option<SessionHandle>,
    pub byte_offset: Option<u64>,
    pub message: String,
    pub details: Value,
}
```

- [ ] **Step 4: Write the sink trait**

`crates/vem-core/src/sink.rs`:

```rust
//! The sink an adapter emits canonical drafts into. The case database implements it with SQL
//! inserts; tests implement it with vectors.

use crate::model::*;

pub trait ParseSink {
    fn session(&mut self, draft: SessionDraft) -> SessionHandle;
    fn update_session(&mut self, session: SessionHandle, update: SessionUpdate);
    /// Looks a session up by its harness id across everything parsed so far in this root.
    fn find_session(&self, harness_session_id: &str) -> Option<SessionHandle>;
    fn message(&mut self, session: SessionHandle, draft: MessageDraft) -> MessageHandle;
    fn tool_call(&mut self, session: SessionHandle, draft: ToolCallDraft) -> ToolCallHandle;
    fn observation(&mut self, session: SessionHandle, draft: ObservationDraft);
    fn identity_claim(&mut self, session: SessionHandle, draft: IdentityClaimDraft);
    fn anomaly(&mut self, draft: AnomalyDraft);
    /// Stores content-addressed bytes and returns their SHA-256 hex.
    fn blob(&mut self, bytes: &[u8]) -> String;
}
```

- [ ] **Step 5: Write the adapter trait**

`crates/vem-core/src/adapter.rs`:

```rust
//! Harness adapter contract (spec §6): identify a root, discover its stores, parse one file.

use crate::model::{Harness, SourceFileHandle};
use crate::sink::ParseSink;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identification {
    pub harness: Harness,
    /// Human-readable signatures that matched, recorded on the evidence root.
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreCandidate {
    /// e.g. `claude:projects`.
    pub kind: String,
    pub generation: Option<String>,
    /// Relative to the root.
    pub rel_path: PathBuf,
    /// Files belonging to this store, relative to the root, sorted.
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub stores: Vec<StoreCandidate>,
    /// Store kinds this harness normally has that were not found.
    pub absent: Vec<String>,
}

pub struct FileContext<'a> {
    pub root: &'a Path,
    pub store: &'a StoreCandidate,
    pub rel_path: &'a Path,
    pub abs_path: PathBuf,
    pub handle: SourceFileHandle,
    pub mtime: Option<SystemTime>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

pub trait HarnessAdapter: Send + Sync {
    fn harness(&self) -> Harness;
    /// `Some` when `root` is this harness's directory, with the signatures that matched.
    fn identify(&self, root: &Path) -> Option<Identification>;
    fn discover(&self, root: &Path) -> Discovery;
    /// Parses one file of one store, emitting into `sink`. Must not panic on malformed input.
    fn parse_file(&self, ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<(), ParseError>;
}
```

- [ ] **Step 6: Write the in-memory test sink**

`crates/vem-core/src/testing.rs`:

```rust
//! `VecSink`: an in-memory `ParseSink` for adapter tests.

use crate::hash::sha256_hex;
use crate::model::*;
use crate::sink::ParseSink;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct VecSink {
    pub sessions: Vec<(SessionHandle, SessionDraft)>,
    pub updates: Vec<(SessionHandle, SessionUpdate)>,
    pub messages: Vec<(SessionHandle, MessageHandle, MessageDraft)>,
    pub tool_calls: Vec<(SessionHandle, ToolCallHandle, ToolCallDraft)>,
    pub observations: Vec<(SessionHandle, ObservationDraft)>,
    pub claims: Vec<(SessionHandle, IdentityClaimDraft)>,
    pub anomalies: Vec<AnomalyDraft>,
    pub blobs: HashMap<String, Vec<u8>>,
}

impl VecSink {
    pub fn messages_with_role(&self, role: Role) -> Vec<&MessageDraft> {
        self.messages.iter().filter(|(_, _, m)| m.role == role).map(|(_, _, m)| m).collect()
    }
    pub fn anomalies_of(&self, kind: AnomalyKind) -> Vec<&AnomalyDraft> {
        self.anomalies.iter().filter(|a| a.kind == kind).collect()
    }
    pub fn observations_of(&self, kind: ObservationKind) -> Vec<&ObservationDraft> {
        self.observations.iter().filter(|(_, o)| o.kind == kind).map(|(_, o)| o).collect()
    }
    pub fn session_draft(&self, h: SessionHandle) -> &SessionDraft {
        &self.sessions.iter().find(|(x, _)| *x == h).expect("session handle").1
    }
}

impl ParseSink for VecSink {
    fn session(&mut self, draft: SessionDraft) -> SessionHandle {
        let h = SessionHandle(self.sessions.len() as i64 + 1);
        self.sessions.push((h, draft));
        h
    }
    fn update_session(&mut self, session: SessionHandle, update: SessionUpdate) {
        self.updates.push((session, update));
    }
    fn find_session(&self, harness_session_id: &str) -> Option<SessionHandle> {
        self.sessions.iter().find(|(_, s)| s.harness_session_id == harness_session_id).map(|(h, _)| *h)
    }
    fn message(&mut self, session: SessionHandle, draft: MessageDraft) -> MessageHandle {
        let h = MessageHandle(self.messages.len() as i64 + 1);
        self.messages.push((session, h, draft));
        h
    }
    fn tool_call(&mut self, session: SessionHandle, draft: ToolCallDraft) -> ToolCallHandle {
        let h = ToolCallHandle(self.tool_calls.len() as i64 + 1);
        self.tool_calls.push((session, h, draft));
        h
    }
    fn observation(&mut self, session: SessionHandle, draft: ObservationDraft) {
        self.observations.push((session, draft));
    }
    fn identity_claim(&mut self, session: SessionHandle, draft: IdentityClaimDraft) {
        self.claims.push((session, draft));
    }
    fn anomaly(&mut self, draft: AnomalyDraft) {
        self.anomalies.push(draft);
    }
    fn blob(&mut self, bytes: &[u8]) -> String {
        let sha = sha256_hex(bytes);
        self.blobs.entry(sha.clone()).or_insert_with(|| bytes.to_vec());
        sha
    }
}
```

- [ ] **Step 7: Run all core tests**

Run: `cargo test -p vem-core`
Expected: all pass (hash 3, jsonl 6, model 4, props 1).

- [ ] **Step 8: Record the two added anomaly kinds in the spec**

In `docs/superpowers/specs/2026-10-08-vem-design.md`, in the §4 `Anomaly` kinds list, change `HashDrift|EmptyStore|OversizedRecord}` to `HashDrift|EmptyStore|OversizedRecord|UnpairedToolResult|MissingTranscript}` and add after the model block:

```markdown
`UnpairedToolResult`: a tool result whose tool use is not in the same file. `MissingTranscript`:
a sidecar (e.g. Claude Code `history.jsonl`) references a session for which no transcript exists,
which is evidence of deletion.
```

- [ ] **Step 9: Commit**

```bash
git add crates/vem-core docs/superpowers/specs
git commit -m "feat(core): canonical model, parse sink and adapter traits

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Claude Code fixture, identification and store discovery

**Files:**
- Create: `fixtures/claude-code/basic/**` (listed below)
- Create: `crates/vem-adapters/src/claude_code/mod.rs`
- Create: `crates/vem-adapters/src/claude_code/discover.rs`
- Create: `crates/vem-adapters/tests/claude_code_discover.rs`
- Modify: `crates/vem-adapters/src/lib.rs`

**Interfaces:**
- Consumes: `vem_core::adapter::{HarnessAdapter, Identification, StoreCandidate, Discovery, FileContext, ParseError}`, `vem_core::model::Harness`.
- Produces: `vem_adapters::claude_code::ClaudeCodeAdapter` (unit struct) implementing `HarnessAdapter`; store kind constants `STORE_PROJECTS = "claude:projects"`, `STORE_HISTORY = "claude:history"`, `STORE_FILE_HISTORY = "claude:file-history"`, plus inventory-only kinds `claude:shell-snapshots`, `claude:todos`, `claude:plans`, `claude:paste-cache`, `claude:uploads`, `claude:settings`; `vem_adapters::adapters() -> Vec<Box<dyn HarnessAdapter>>`, `vem_adapters::identify_root(&Path) -> Vec<Identification>`, `vem_adapters::adapter_for(Harness) -> Option<Box<dyn HarnessAdapter>>`; `vem_adapters::claude_code::discover::list_files(root, rel) -> Vec<PathBuf>` (sorted, relative to root).
- The fixture `fixtures/claude-code/basic` is used by every later task. Session S1 = `0f0f0f0f-0000-4000-8000-000000000001`, session S0 = `0f0f0f0f-0000-4000-8000-000000000000`, subagent `agent-0123456789abcdef`.

- [ ] **Step 1: Create the fixture**

Create the directory tree with exactly these files. JSONL files must have one JSON object per line; the last line of S1's transcript is deliberately truncated and has **no trailing newline**.

`fixtures/claude-code/basic/settings.json`:
```json
{"permissions":{"allow":["Bash(git status)"]},"model":"claude-fable-5-1"}
```

`fixtures/claude-code/basic/projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl` (17 lines; write with a tool that does not append a final newline, e.g. `printf`):
```
{"type":"ai-title","aiTitle":"Add notes file","sessionId":"0f0f0f0f-0000-4000-8000-000000000001"}
{"type":"last-prompt","leafUuid":"a5a5a5a5-0000-4000-8000-000000000005","sessionId":"0f0f0f0f-0000-4000-8000-000000000001"}
{"parentUuid":null,"isSidechain":false,"promptId":"p1","type":"user","message":{"role":"user","content":"Create a notes file and list the repo"},"uuid":"u1u1u1u1-0000-4000-8000-000000000001","timestamp":"2026-09-30T10:00:00.000Z","permissionMode":"default","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"u1u1u1u1-0000-4000-8000-000000000001","isSidechain":false,"message":{"model":"claude-fable-5-1","id":"msg_1","type":"message","role":"assistant","content":[{"type":"thinking","thinking":"List first.","signature":"sig"},{"type":"tool_use","id":"toolu_bash1","name":"Bash","input":{"command":"ls -la","description":"List repo"}}],"stop_reason":"tool_use"},"requestId":"req_1","type":"assistant","uuid":"a1a1a1a1-0000-4000-8000-000000000001","timestamp":"2026-09-30T10:00:01.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"a1a1a1a1-0000-4000-8000-000000000001","isSidechain":false,"promptId":"p1","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_bash1","type":"tool_result","content":"README.md\nsrc\n","is_error":false}]},"uuid":"u2u2u2u2-0000-4000-8000-000000000002","timestamp":"2026-09-30T10:00:02.000Z","toolUseResult":{"stdout":"README.md\nsrc\n","stderr":"","interrupted":false,"isImage":false},"sourceToolAssistantUUID":"a1a1a1a1-0000-4000-8000-000000000001","session_id":"0f0f0f0f-0000-4000-8000-000000000001","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"u2u2u2u2-0000-4000-8000-000000000002","isSidechain":false,"message":{"model":"claude-fable-5-1","id":"msg_2","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_write1","name":"Write","input":{"file_path":"/home/alice/proj/notes.md","content":"# Notes\n"}}],"stop_reason":"tool_use"},"requestId":"req_2","type":"assistant","uuid":"a2a2a2a2-0000-4000-8000-000000000002","timestamp":"2026-09-30T10:00:03.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"a2a2a2a2-0000-4000-8000-000000000002","isSidechain":false,"promptId":"p1","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_write1","type":"tool_result","content":"File created successfully at: /home/alice/proj/notes.md"}]},"uuid":"u3u3u3u3-0000-4000-8000-000000000003","timestamp":"2026-09-30T10:00:04.000Z","toolUseResult":{"type":"create","filePath":"/home/alice/proj/notes.md","content":"# Notes\n","structuredPatch":[],"originalFile":null,"userModified":false},"sourceToolAssistantUUID":"a2a2a2a2-0000-4000-8000-000000000002","session_id":"0f0f0f0f-0000-4000-8000-000000000001","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"type":"file-history-snapshot","messageId":"u3u3u3u3-0000-4000-8000-000000000003","snapshot":{"messageId":"u3u3u3u3-0000-4000-8000-000000000003","trackedFileBackups":{"notes.md":{"backupFileName":"deadbeef00000001@v1","version":1,"backupTime":"2026-09-30T10:00:04.500Z","realParentDir":"/home/alice/proj"}},"timestamp":"2026-09-30T10:00:04.500Z"},"isSnapshotUpdate":false}
{"type":"file-history-delta","messageId":"u3u3u3u3-0000-4000-8000-000000000003","snapshotMessageId":"u3u3u3u3-0000-4000-8000-000000000003","trackingPath":"notes.md","backup":{"backupFileName":"deadbeef00000001@v1","version":1,"backupTime":"2026-09-30T10:00:04.500Z","realParentDir":"/home/alice/proj"},"timestamp":"2026-09-30T10:00:04.500Z"}
{"parentUuid":"u3u3u3u3-0000-4000-8000-000000000003","isSidechain":false,"message":{"model":"claude-fable-5-1","id":"msg_3","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_edit1","name":"Edit","input":{"file_path":"/home/alice/proj/notes.md","old_string":"# Notes\n","new_string":"# Notes\n\n- first\n"}}],"stop_reason":"tool_use"},"requestId":"req_3","type":"assistant","uuid":"a3a3a3a3-0000-4000-8000-000000000003","timestamp":"2026-09-30T10:00:05.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"a3a3a3a3-0000-4000-8000-000000000003","isSidechain":false,"promptId":"p1","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_edit1","type":"tool_result","content":"The file /home/alice/proj/notes.md has been updated successfully."}]},"uuid":"u4u4u4u4-0000-4000-8000-000000000004","timestamp":"2026-09-30T10:00:06.000Z","toolUseResult":{"filePath":"/home/alice/proj/notes.md","oldString":"# Notes\n","newString":"# Notes\n\n- first\n","originalFile":"# Notes\n","structuredPatch":[{"oldStart":1,"oldLines":1,"newStart":1,"newLines":3,"lines":[" # Notes","+","+- first"]}],"userModified":false,"replaceAll":false},"sourceToolAssistantUUID":"a3a3a3a3-0000-4000-8000-000000000003","session_id":"0f0f0f0f-0000-4000-8000-000000000000","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"u4u4u4u4-0000-4000-8000-000000000004","isSidechain":false,"message":{"model":"claude-fable-5-1","id":"msg_4","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_agent1","name":"Agent","input":{"subagent_type":"Explore","description":"Find config files","prompt":"Find all config files in the repo"}}],"stop_reason":"tool_use"},"requestId":"req_4","type":"assistant","uuid":"a4a4a4a4-0000-4000-8000-000000000004","timestamp":"2026-09-30T10:00:07.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"a4a4a4a4-0000-4000-8000-000000000004","isSidechain":false,"promptId":"p1","type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_agent1","type":"tool_result","content":[{"type":"text","text":"Found settings.json"}]}]},"uuid":"u5u5u5u5-0000-4000-8000-000000000005","timestamp":"2026-09-30T10:00:09.000Z","toolUseResult":{"agentId":"0123456789abcdef","status":"completed","content":[{"type":"text","text":"Found settings.json"}]},"sourceToolAssistantUUID":"a4a4a4a4-0000-4000-8000-000000000004","session_id":"0f0f0f0f-0000-4000-8000-000000000001","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"u5u5u5u5-0000-4000-8000-000000000005","isSidechain":false,"message":{"model":"claude-fable-5-1","id":"msg_5","type":"message","role":"assistant","content":[{"type":"text","text":"Done. Created notes.md and added a first item."}],"stop_reason":"end_turn"},"requestId":"req_5","type":"assistant","uuid":"a5a5a5a5-0000-4000-8000-000000000005","timestamp":"2026-09-30T10:00:10.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"a5a5a5a5-0000-4000-8000-000000000005","isSidechain":false,"type":"system","subtype":"turn_duration","durationMs":10000,"messageCount":10,"timestamp":"2026-09-30T10:00:10.100Z","uuid":"s1s1s1s1-0000-4000-8000-000000000001","isMeta":false,"userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"type":"zz-future-record","payload":{"x":1},"sessionId":"0f0f0f0f-0000-4000-8000-000000000001"}
{"parentUuid":"s1s1s1s1-0000-4000-8000-000000000001","isSidechain":false,"type":"user","message":{"role":"user","content":"this line was cut off mid-wri
```

`fixtures/claude-code/basic/projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000000.jsonl` (2 lines, trailing newline):
```
{"parentUuid":null,"isSidechain":false,"promptId":"p0","type":"user","message":{"role":"user","content":"Earlier session"},"uuid":"u0u0u0u0-0000-4000-8000-000000000000","timestamp":"2026-09-29T09:00:00.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000000","version":"2.1.290","gitBranch":"main"}
{"parentUuid":"u0u0u0u0-0000-4000-8000-000000000000","isSidechain":false,"message":{"model":"claude-fable-5-1","id":"msg_0","type":"message","role":"assistant","content":[{"type":"text","text":"Hello from the earlier session."}],"stop_reason":"end_turn"},"requestId":"req_0","type":"assistant","uuid":"a0a0a0a0-0000-4000-8000-000000000000","timestamp":"2026-09-29T09:00:01.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000000","version":"2.1.290","gitBranch":"main"}
```

`fixtures/claude-code/basic/projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/subagents/agent-0123456789abcdef.jsonl` (2 lines, trailing newline):
```
{"parentUuid":null,"isSidechain":true,"promptId":"p1","agentId":"0123456789abcdef","type":"user","message":{"role":"user","content":"Find all config files in the repo"},"uuid":"b1b1b1b1-0000-4000-8000-000000000001","timestamp":"2026-09-30T10:00:07.200Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
{"parentUuid":"b1b1b1b1-0000-4000-8000-000000000001","isSidechain":true,"agentId":"0123456789abcdef","message":{"model":"claude-haiku-5-5","id":"msg_b2","type":"message","role":"assistant","content":[{"type":"text","text":"Found settings.json"}],"stop_reason":"end_turn"},"requestId":"req_b2","type":"assistant","uuid":"b2b2b2b2-0000-4000-8000-000000000002","timestamp":"2026-09-30T10:00:08.500Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001","version":"2.1.294","gitBranch":"main"}
```

`fixtures/claude-code/basic/projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/subagents/agent-0123456789abcdef.meta.json`:
```json
{"agentType":"Explore","description":"Find config files","toolUseId":"toolu_agent1","spawnDepth":1,"requestShape":"background","requestNonInteractive":true}
```

`fixtures/claude-code/basic/projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/tool-results/bo425nxqk.txt`:
```
README.md
src
```

`fixtures/claude-code/basic/projects/-home-alice-proj/22222222-0000-4000-8000-000000000002.jsonl.orphaned-1759221000000` (1 line, trailing newline):
```
{"parentUuid":null,"isSidechain":false,"promptId":"p2","type":"user","message":{"role":"user","content":"orphaned session prompt"},"uuid":"u9u9u9u9-0000-4000-8000-000000000009","timestamp":"2026-09-30T11:00:00.000Z","userType":"external","entrypoint":"cli","cwd":"/home/alice/proj","sessionId":"22222222-0000-4000-8000-000000000002","version":"2.1.294","gitBranch":"main"}
```

`fixtures/claude-code/basic/history.jsonl` (3 lines, trailing newline):
```
{"display":"Create a notes file and list the repo","pastedContents":{},"timestamp":1790848800000,"project":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001"}
{"display":"[Pasted text #1 +3 lines] please review","pastedContents":{"1":{"id":1,"type":"text","content":"AKIAIOSFODNN7EXAMPLE\nline2\nline3"}},"timestamp":1790848860000,"project":"/home/alice/proj","sessionId":"0f0f0f0f-0000-4000-8000-000000000001"}
{"display":"a prompt from a deleted session","pastedContents":{},"timestamp":1790700000000,"project":"/home/alice/other","sessionId":"33333333-0000-4000-8000-000000000003"}
```

`fixtures/claude-code/basic/file-history/0f0f0f0f-0000-4000-8000-000000000001/deadbeef00000001@v1` (exactly the 8 bytes `# Notes` followed by one newline; tests hash it):
```
# Notes
```

`fixtures/claude-code/basic/shell-snapshots/snapshot-bash-1790848800000-abc123.sh`:
```
export PATH=/usr/bin
```

Verify the truncated transcript has no final newline:
```bash
tail -c 1 fixtures/claude-code/basic/projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl | xxd
```
Expected: the last byte is `69` (`i`), not `0a`.

- [ ] **Step 2: Write the failing discovery tests**

`crates/vem-adapters/tests/claude_code_discover.rs`:

```rust
use std::path::{Path, PathBuf};
use vem_core::adapter::HarnessAdapter;
use vem_core::model::Harness;
use vem_adapters::claude_code::{ClaudeCodeAdapter, STORE_FILE_HISTORY, STORE_HISTORY, STORE_PROJECTS};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/claude-code/basic").canonicalize().unwrap()
}

#[test]
fn identifies_fixture_as_claude_code_by_content() {
    let id = ClaudeCodeAdapter.identify(&fixture()).expect("identified");
    assert_eq!(id.harness, Harness::ClaudeCode);
    assert!(id.evidence.iter().any(|e| e.contains("parentUuid")), "{:?}", id.evidence);
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
    assert_eq!(kinds[0], STORE_PROJECTS, "projects store must come first so sidecars can find sessions");
    assert!(kinds.contains(&STORE_HISTORY));
    assert!(kinds.contains(&STORE_FILE_HISTORY));
    assert!(kinds.contains(&"claude:shell-snapshots"));
    assert!(kinds.contains(&"claude:settings"));
    assert!(d.absent.contains(&"claude:todos".to_string()));
    assert!(d.absent.contains(&"claude:plans".to_string()));
    let projects = &d.stores[0];
    assert_eq!(projects.generation.as_deref(), Some("2.1.294"));
    let files: Vec<String> = projects.files.iter().map(|p| p.to_string_lossy().to_string()).collect();
    assert!(files.contains(&"projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl".to_string()));
    assert!(files.contains(&"projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/subagents/agent-0123456789abcdef.jsonl".to_string()));
    assert!(files.contains(&"projects/-home-alice-proj/22222222-0000-4000-8000-000000000002.jsonl.orphaned-1759221000000".to_string()));
    assert!(files.windows(2).all(|w| w[0] <= w[1]), "files must be sorted");
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
```

Add `walkdir.workspace = true` under `[dev-dependencies]` in `crates/vem-adapters/Cargo.toml` (it is already a normal dependency; dev-dependency entry is not needed, tests can use normal dependencies; skip this).

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p vem-adapters --test claude_code_discover`
Expected: compile error, `claude_code` module missing.

- [ ] **Step 4: Implement the registry and adapter skeleton**

`crates/vem-adapters/src/lib.rs`:

```rust
//! Harness adapters. Each adapter is a pure function from evidence files to canonical drafts.

pub mod claude_code;

use std::path::Path;
use vem_core::adapter::{HarnessAdapter, Identification};
use vem_core::model::Harness;

pub fn adapters() -> Vec<Box<dyn HarnessAdapter>> {
    vec![Box::new(claude_code::ClaudeCodeAdapter)]
}

pub fn adapter_for(harness: Harness) -> Option<Box<dyn HarnessAdapter>> {
    adapters().into_iter().find(|a| a.harness() == harness)
}

/// Every harness whose content signatures match `root`. Zero means unrecognized, more than one means ambiguous.
pub fn identify_root(root: &Path) -> Vec<Identification> {
    adapters().iter().filter_map(|a| a.identify(root)).collect()
}
```

`crates/vem-adapters/src/claude_code/mod.rs`:

```rust
//! Claude Code adapter: a collected `.claude` directory (spec §6.1).

pub mod discover;

use std::path::Path;
use vem_core::adapter::{Discovery, FileContext, HarnessAdapter, Identification, ParseError};
use vem_core::model::Harness;
use vem_core::sink::ParseSink;

pub const STORE_PROJECTS: &str = "claude:projects";
pub const STORE_HISTORY: &str = "claude:history";
pub const STORE_FILE_HISTORY: &str = "claude:file-history";

/// (store kind, path relative to root). Order matters: `projects` first so sidecars can find sessions.
pub const EXPECTED_STORES: &[(&str, &str)] = &[
    (STORE_PROJECTS, "projects"),
    (STORE_HISTORY, "history.jsonl"),
    (STORE_FILE_HISTORY, "file-history"),
    ("claude:shell-snapshots", "shell-snapshots"),
    ("claude:todos", "todos"),
    ("claude:plans", "plans"),
    ("claude:paste-cache", "paste-cache"),
    ("claude:uploads", "uploads"),
    ("claude:settings", "settings.json"),
];

pub struct ClaudeCodeAdapter;

impl HarnessAdapter for ClaudeCodeAdapter {
    fn harness(&self) -> Harness {
        Harness::ClaudeCode
    }

    fn identify(&self, root: &Path) -> Option<Identification> {
        discover::identify(root)
    }

    fn discover(&self, root: &Path) -> Discovery {
        discover::discover(root)
    }

    fn parse_file(&self, _ctx: &FileContext<'_>, _sink: &mut dyn ParseSink) -> Result<(), ParseError> {
        Ok(())
    }
}
```

`crates/vem-adapters/src/claude_code/discover.rs`:

```rust
//! Identification by content signature and store discovery for a `.claude` directory.

use super::EXPECTED_STORES;
use std::io::Read;
use std::path::{Path, PathBuf};
use vem_core::adapter::{Discovery, Identification, StoreCandidate};
use vem_core::model::Harness;

const SIGNATURE_BYTES: usize = 16 * 1024;

fn head(path: &Path) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(SIGNATURE_BYTES as u64).read_to_end(&mut buf);
    }
    buf
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

pub fn is_transcript_name(name: &str) -> bool {
    name.ends_with(".jsonl") || name.contains(".jsonl.orphaned-") || name.contains(".jsonl.superseded-")
}

/// First transcript under `projects/` whose head carries both `parentUuid` and `sessionId`.
fn first_transcript_signature(root: &Path) -> Option<PathBuf> {
    let projects = root.join("projects");
    if !projects.is_dir() {
        return None;
    }
    for entry in walkdir::WalkDir::new(&projects).max_depth(4).sort_by_file_name().into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !is_transcript_name(&name) {
            continue;
        }
        let h = head(entry.path());
        if contains(&h, "\"parentUuid\"") && contains(&h, "\"sessionId\"") {
            return entry.path().strip_prefix(root).ok().map(Path::to_path_buf);
        }
    }
    None
}

fn history_signature(root: &Path) -> bool {
    let h = head(&root.join("history.jsonl"));
    contains(&h, "\"display\"") && contains(&h, "\"sessionId\"")
}

pub fn identify(root: &Path) -> Option<Identification> {
    let mut evidence = Vec::new();
    if let Some(p) = first_transcript_signature(root) {
        evidence.push(format!("projects transcript carrying parentUuid and sessionId: {}", p.display()));
    }
    if history_signature(root) {
        evidence.push("history.jsonl carrying display and sessionId".to_string());
    }
    if root.join("file-history").is_dir() {
        evidence.push("file-history directory".to_string());
    }
    if evidence.is_empty() {
        None
    } else {
        Some(Identification { harness: Harness::ClaudeCode, evidence })
    }
}

/// All regular files under `root/rel`, as paths relative to `root`, sorted. A file path yields itself.
pub fn list_files(root: &Path, rel: &Path) -> Vec<PathBuf> {
    let abs = root.join(rel);
    let mut out = Vec::new();
    if abs.is_file() {
        out.push(rel.to_path_buf());
        return out;
    }
    for entry in walkdir::WalkDir::new(&abs).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_file() {
            if let Ok(r) = entry.path().strip_prefix(root) {
                out.push(r.to_path_buf());
            }
        }
    }
    out.sort();
    out
}

/// The harness version seen in the first transcript record that carries one.
fn detect_generation(root: &Path, files: &[PathBuf]) -> Option<String> {
    for f in files {
        let name = f.file_name()?.to_string_lossy().to_string();
        if !is_transcript_name(&name) {
            continue;
        }
        let h = head(&root.join(f));
        for line in h.split(|&b| b == b'\n') {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) {
                if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
                    return Some(ver.to_string());
                }
            }
        }
    }
    None
}

pub fn discover(root: &Path) -> Discovery {
    let mut d = Discovery::default();
    for (kind, rel) in EXPECTED_STORES {
        let rel_path = PathBuf::from(rel);
        if !root.join(&rel_path).exists() {
            d.absent.push(kind.to_string());
            continue;
        }
        let files = list_files(root, &rel_path);
        let generation = if *kind == super::STORE_PROJECTS { detect_generation(root, &files) } else { None };
        d.stores.push(StoreCandidate { kind: kind.to_string(), generation, rel_path, files });
    }
    d
}
```

- [ ] **Step 5: Run the discovery tests**

Run: `cargo test -p vem-adapters --test claude_code_discover`
Expected: 5 passed.

- [ ] **Step 6: Commit**

```bash
git add fixtures crates/vem-adapters
git commit -m "feat(adapters): claude code fixture, content-signature identification and store discovery

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Claude Code transcript parser (messages, blocks, meta, anomalies, provenance)

**Files:**
- Create: `crates/vem-adapters/src/claude_code/transcript.rs`
- Create: `crates/vem-adapters/tests/claude_code_transcript.rs`
- Create: `crates/vem-adapters/tests/common/mod.rs`
- Modify: `crates/vem-adapters/src/claude_code/mod.rs`

**Interfaces:**
- Consumes: `vem_core::jsonl::JsonlReader`, `vem_core::model::*`, `vem_core::sink::ParseSink`, `vem_core::adapter::{FileContext, ParseError}`, `super::discover::is_transcript_name`.
- Produces: `transcript::parse_transcript(ctx: &FileContext, sink: &mut dyn ParseSink) -> Result<(), ParseError>`; `transcript::classify_path(rel: &Path) -> Option<TranscriptPath { session_id, is_subagent, parent_session_id, flags }>`; `transcript::provenance(handle, &RawRecord) -> Provenance`; `transcript::blocks_from_content(Option<&Value>) -> Vec<BlockDraft>`; `transcript::flatten_text(Option<&Value>) -> String`; `transcript::str_field(&Value, &str) -> Option<String>`; `transcript::PARSER_NAME`, `PARSER_VERSION`; the `TranscriptState` struct with `pub(crate)` fields `root`, `handle`, `session`, `session_id`, `pending: HashMap<String, PendingToolUse>` and `PendingToolUse { message, ordinal, name, input, started }` that Task 7 extends.
- Test helper `tests/common/mod.rs::parse_fixture(rel: &str) -> VecSink` used by Tasks 7, 8, 9.

- [ ] **Step 1: Write the test helper and failing tests**

`crates/vem-adapters/tests/common/mod.rs`:

```rust
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use vem_core::adapter::{FileContext, HarnessAdapter};
use vem_core::model::SourceFileHandle;
use vem_core::testing::VecSink;
use vem_adapters::claude_code::ClaudeCodeAdapter;

pub fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/claude-code/basic").canonicalize().unwrap()
}

pub const S1: &str = "0f0f0f0f-0000-4000-8000-000000000001";
pub const S0: &str = "0f0f0f0f-0000-4000-8000-000000000000";
pub const S1_FILE: &str = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl";
pub const S0_FILE: &str = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000000.jsonl";
pub const SUB_FILE: &str = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/subagents/agent-0123456789abcdef.jsonl";
pub const ORPHAN_FILE: &str = "projects/-home-alice-proj/22222222-0000-4000-8000-000000000002.jsonl.orphaned-1759221000000";

/// Parses one file of `root` into a fresh `VecSink`, giving it source-file handle 1.
pub fn parse_file_into(root: &Path, rel: &str, sink: &mut VecSink) {
    let adapter = ClaudeCodeAdapter;
    let discovery = adapter.discover(root);
    let rel_path = PathBuf::from(rel);
    let store = discovery
        .stores
        .iter()
        .find(|s| s.files.contains(&rel_path))
        .unwrap_or_else(|| panic!("no store claims {rel}"));
    let abs_path = root.join(&rel_path);
    let mtime = std::fs::metadata(&abs_path).and_then(|m| m.modified()).ok();
    let ctx = FileContext { root, store, rel_path: &rel_path, abs_path, handle: SourceFileHandle(1), mtime };
    adapter.parse_file(&ctx, sink).expect("parse ok");
}

pub fn parse_fixture(rel: &str) -> VecSink {
    let mut sink = VecSink::default();
    parse_file_into(&fixture_root(), rel, &mut sink);
    sink
}

/// Writes `lines` as a transcript named `<session>.jsonl` under a temporary `.claude`-shaped root.
pub fn temp_root_with_transcript(session: &str, lines: &[&str], trailing_newline: bool) -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects").join("-tmp-proj");
    std::fs::create_dir_all(&dir).unwrap();
    let mut body = lines.join("\n");
    if trailing_newline {
        body.push('\n');
    }
    let rel = format!("projects/-tmp-proj/{session}.jsonl");
    std::fs::write(tmp.path().join(&rel), body).unwrap();
    (tmp, rel)
}
```

`crates/vem-adapters/tests/claude_code_transcript.rs`:

```rust
mod common;

use common::*;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::testing::VecSink;

#[test]
fn parses_primary_session_messages_and_roles() {
    let sink = parse_fixture(S1_FILE);
    assert_eq!(sink.sessions.len(), 1);
    let (_, s) = &sink.sessions[0];
    assert_eq!(s.harness_session_id, S1);
    assert_eq!(s.kind, SessionKind::Primary);
    assert_eq!(sink.messages.len(), 16);
    assert_eq!(sink.messages_with_role(Role::User).len(), 1);
    assert_eq!(sink.messages_with_role(Role::Assistant).len(), 5);
    assert_eq!(sink.messages_with_role(Role::Tool).len(), 4);
    assert_eq!(sink.messages_with_role(Role::System).len(), 1);
    assert_eq!(sink.messages_with_role(Role::Meta).len(), 5);
}

#[test]
fn blocks_carry_kinds_text_and_tool_use_ids() {
    let sink = parse_fixture(S1_FILE);
    let user = &sink.messages_with_role(Role::User)[0];
    assert_eq!(user.blocks.len(), 1);
    assert_eq!(user.blocks[0].kind, BlockKind::Text);
    assert_eq!(user.blocks[0].text.as_deref(), Some("Create a notes file and list the repo"));
    assert_eq!(user.harness_uuid.as_deref(), Some("u1u1u1u1-0000-4000-8000-000000000001"));
    assert_eq!(user.parent_uuid, None);
    assert_eq!(user.timestamp.value.as_deref(), Some("2026-09-30T10:00:00.000Z"));
    assert_eq!(user.timestamp.origin, TsOrigin::Stored);

    let a1 = &sink.messages_with_role(Role::Assistant)[0];
    assert_eq!(a1.model.as_deref(), Some("claude-fable-5-1"));
    assert_eq!(a1.blocks[0].kind, BlockKind::Thinking);
    assert_eq!(a1.blocks[0].text.as_deref(), Some("List first."));
    assert_eq!(a1.blocks[1].kind, BlockKind::ToolUse);
    assert_eq!(a1.blocks[1].tool_use_id.as_deref(), Some("toolu_bash1"));

    let t1 = &sink.messages_with_role(Role::Tool)[0];
    assert_eq!(t1.blocks[0].kind, BlockKind::ToolResult);
    assert_eq!(t1.blocks[0].tool_use_id.as_deref(), Some("toolu_bash1"));
    assert_eq!(t1.blocks[0].text.as_deref(), Some("README.md\nsrc\n"));
    assert!(t1.attributes.contains_key("cwd"));
    assert!(!t1.attributes.contains_key("message"));
    assert!(!t1.attributes.contains_key("toolUseResult"));
}

#[test]
fn meta_records_and_unknown_types_are_kept() {
    let sink = parse_fixture(S1_FILE);
    let meta = sink.messages_with_role(Role::Meta);
    let types: Vec<&str> = meta.iter().map(|m| m.harness_record_type.as_str()).collect();
    assert_eq!(types, vec!["ai-title", "last-prompt", "file-history-snapshot", "file-history-delta", "zz-future-record"]);
    let unknown = meta[4];
    assert_eq!(unknown.attributes.get("payload").unwrap(), &serde_json::json!({"x": 1}));
    let anomalies = sink.anomalies_of(AnomalyKind::UnknownRecordType);
    assert_eq!(anomalies.len(), 1);
    assert_eq!(anomalies[0].severity, Severity::Info);
    assert!(anomalies[0].message.contains("zz-future-record"));
}

#[test]
fn truncated_final_line_is_an_anomaly_with_offset() {
    let sink = parse_fixture(S1_FILE);
    let a = sink.anomalies_of(AnomalyKind::TruncatedLine);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].severity, Severity::Warning);
    assert_eq!(a[0].source_file, Some(SourceFileHandle(1)));
    assert!(a[0].byte_offset.unwrap() > 0);
    assert!(sink.anomalies_of(AnomalyKind::MalformedRecord).is_empty());
}

#[test]
fn provenance_points_at_the_exact_bytes() {
    let sink = parse_fixture(S1_FILE);
    let raw = std::fs::read(fixture_root().join(S1_FILE)).unwrap();
    let first_line_len = raw.iter().position(|&b| b == b'\n').unwrap();
    let (_, _, m0) = &sink.messages[0];
    assert_eq!(m0.provenance.byte_offset, 0);
    assert_eq!(m0.provenance.byte_length, first_line_len as u64);
    assert_eq!(m0.provenance.record_index, 0);
    assert_eq!(m0.provenance.content_sha256, sha256_hex(&raw[..first_line_len]));
    assert_eq!(m0.provenance.parser_name, "claude_code.transcript");
    assert_eq!(m0.provenance.origin, ProvOrigin::Stored);
    for (_, _, m) in &sink.messages {
        let slice = &raw[m.provenance.byte_offset as usize..(m.provenance.byte_offset + m.provenance.byte_length) as usize];
        assert_eq!(sha256_hex(slice), m.provenance.content_sha256);
    }
}

#[test]
fn session_fields_are_learned_from_records() {
    let sink = parse_fixture(S1_FILE);
    let merged = sink.updates.iter().fold(SessionUpdate::default(), |mut acc, (_, u)| {
        if u.title.is_some() { acc.title = u.title.clone(); }
        if acc.project_path.is_none() { acc.project_path = u.project_path.clone(); }
        if acc.git_branch.is_none() { acc.git_branch = u.git_branch.clone(); }
        if acc.harness_version.is_none() { acc.harness_version = u.harness_version.clone(); }
        if acc.model.is_none() { acc.model = u.model.clone(); }
        acc
    });
    assert_eq!(merged.title.as_deref(), Some("Add notes file"));
    assert_eq!(merged.project_path.as_deref(), Some("/home/alice/proj"));
    assert_eq!(merged.git_branch.as_deref(), Some("main"));
    assert_eq!(merged.harness_version.as_deref(), Some("2.1.294"));
    assert_eq!(merged.model.as_deref(), Some("claude-fable-5-1"));
}

#[test]
fn snapshot_of_all_messages() {
    let sink = parse_fixture(S1_FILE);
    insta::assert_json_snapshot!("s1_messages", sink.messages);
}

#[test]
fn missing_timestamp_on_conversation_record_is_flagged_and_offsets_are_normalized() {
    let (tmp, rel) = temp_root_with_transcript(
        "aaaaaaaa-0000-4000-8000-00000000000a",
        &[
            r#"{"type":"user","message":{"role":"user","content":"no time"},"uuid":"x1","sessionId":"aaaaaaaa-0000-4000-8000-00000000000a"}"#,
            r#"{"type":"user","message":{"role":"user","content":"offset"},"uuid":"x2","timestamp":"2026-09-30T12:00:00.000+02:00","sessionId":"aaaaaaaa-0000-4000-8000-00000000000a"}"#,
            r#"{"type":"user","message":{"role":"user","content":"garbage time"},"uuid":"x3","timestamp":"yesterday","sessionId":"aaaaaaaa-0000-4000-8000-00000000000a"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.messages.len(), 3);
    assert_eq!(sink.messages[0].2.timestamp, Timestamp::absent());
    assert_eq!(sink.messages[1].2.timestamp.value.as_deref(), Some("2026-09-30T10:00:00.000Z"));
    assert_eq!(sink.messages[2].2.timestamp.origin, TsOrigin::Absent);
    assert_eq!(sink.anomalies_of(AnomalyKind::MissingTimestamp).len(), 2);
}

#[test]
fn malformed_terminated_line_is_an_error_anomaly_and_parsing_continues() {
    let (tmp, rel) = temp_root_with_transcript(
        "bbbbbbbb-0000-4000-8000-00000000000b",
        &[
            r#"{"type":"user","message":{"role":"user","content":"ok"},"uuid":"y1","timestamp":"2026-09-30T10:00:00Z","sessionId":"bbbbbbbb-0000-4000-8000-00000000000b"}"#,
            r#"{"type":"user","message":{"role":"user","con"#,
            r#"{"type":"user","message":{"role":"user","content":"after"},"uuid":"y3","timestamp":"2026-09-30T10:00:02Z","sessionId":"bbbbbbbb-0000-4000-8000-00000000000b"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.messages.len(), 2);
    let a = sink.anomalies_of(AnomalyKind::MalformedRecord);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].severity, Severity::Error);
}

#[test]
fn odd_content_shapes_never_panic() {
    let (tmp, rel) = temp_root_with_transcript(
        "cccccccc-0000-4000-8000-00000000000c",
        &[
            r#"{"type":"user","message":{"role":"user"},"uuid":"z1","timestamp":"2026-09-30T10:00:00Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
            r#"{"type":"user","message":{"role":"user","content":[]},"uuid":"z2","timestamp":"2026-09-30T10:00:01Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
            r#"{"type":"user","message":{"role":"user","content":42},"uuid":"z3","timestamp":"2026-09-30T10:00:02Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
            r#"{"type":"assistant","uuid":"z4","timestamp":"2026-09-30T10:00:03Z","sessionId":"cccccccc-0000-4000-8000-00000000000c"}"#,
        ],
        true,
    );
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), &rel, &mut sink);
    assert_eq!(sink.messages.len(), 4);
    assert!(sink.messages[0].2.blocks.is_empty());
    assert!(sink.messages[1].2.blocks.is_empty());
    assert_eq!(sink.messages[2].2.blocks[0].kind, BlockKind::Other);
    assert_eq!(sink.messages[2].2.role, Role::User);
    assert_eq!(sink.messages[3].2.role, Role::Assistant);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-adapters --test claude_code_transcript`
Expected: tests compile (the adapter's `parse_file` is a no-op) and fail on `sink.sessions.len() == 1`.

- [ ] **Step 3: Implement the transcript parser**

`crates/vem-adapters/src/claude_code/transcript.rs`:

```rust
//! Parser for `projects/<cwd>/<session>.jsonl` transcripts and their `subagents/` siblings (spec §6.1).

use super::discover::is_transcript_name;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use vem_core::adapter::{FileContext, ParseError};
use vem_core::hash::sha256_hex;
use vem_core::jsonl::{JsonlReader, RawRecord};
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const PARSER_NAME: &str = "claude_code.transcript";
pub const PARSER_VERSION: &str = "1";

/// Record types that are session bookkeeping, not conversation. They become `meta` messages.
const KNOWN_META: &[&str] = &[
    "attachment", "summary", "ai-title", "custom-title", "last-prompt", "mode", "permission-mode",
    "atis-latch", "bridge-session", "file-history-snapshot", "file-history-delta", "queue-operation",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptPath {
    pub session_id: String,
    pub is_subagent: bool,
    pub parent_session_id: Option<String>,
    /// `orphaned` and/or `superseded` when the file name carries those markers.
    pub flags: Vec<&'static str>,
}

/// `projects/<cwd>/<uuid>.jsonl[.orphaned-N]` or `projects/<cwd>/<uuid>/subagents/agent-<id>.jsonl`.
pub fn classify_path(rel: &Path) -> Option<TranscriptPath> {
    let name = rel.file_name()?.to_str()?;
    if !is_transcript_name(name) {
        return None;
    }
    let session_id = name.split(".jsonl").next()?.to_string();
    let mut flags = Vec::new();
    if name.contains(".orphaned-") {
        flags.push("orphaned");
    }
    if name.contains(".superseded-") {
        flags.push("superseded");
    }
    let comps: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let n = comps.len();
    let is_subagent = n >= 2 && comps[n - 2] == "subagents";
    let parent_session_id = if is_subagent && n >= 3 { Some(comps[n - 3].clone()) } else { None };
    Some(TranscriptPath { session_id, is_subagent, parent_session_id, flags })
}

pub fn provenance(handle: SourceFileHandle, rec: &RawRecord) -> Provenance {
    Provenance {
        source_file: handle,
        byte_offset: rec.offset,
        byte_length: rec.length,
        record_index: rec.index,
        content_sha256: sha256_hex(&rec.bytes),
        parser_name: PARSER_NAME.to_string(),
        parser_version: PARSER_VERSION.to_string(),
        origin: ProvOrigin::Stored,
    }
}

pub fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Text of a `content` value: a string, or the `text` of each item in an array, joined by newlines.
pub fn flatten_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn block_from_item(item: &Value) -> BlockDraft {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "text" => BlockDraft { kind: BlockKind::Text, text: str_field(item, "text"), payload: item.clone(), tool_use_id: None },
        "thinking" => BlockDraft { kind: BlockKind::Thinking, text: str_field(item, "thinking"), payload: item.clone(), tool_use_id: None },
        "tool_use" => BlockDraft { kind: BlockKind::ToolUse, text: None, payload: item.clone(), tool_use_id: str_field(item, "id") },
        "tool_result" => BlockDraft {
            kind: BlockKind::ToolResult,
            text: Some(flatten_text(item.get("content"))),
            payload: item.clone(),
            tool_use_id: str_field(item, "tool_use_id"),
        },
        "image" => BlockDraft { kind: BlockKind::Image, text: None, payload: item.clone(), tool_use_id: None },
        _ => BlockDraft { kind: BlockKind::Other, text: None, payload: item.clone(), tool_use_id: None },
    }
}

pub fn blocks_from_content(content: Option<&Value>) -> Vec<BlockDraft> {
    match content {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![BlockDraft::text(s)],
        Some(Value::Array(items)) => items.iter().map(block_from_item).collect(),
        Some(other) => vec![BlockDraft { kind: BlockKind::Other, text: None, payload: other.clone(), tool_use_id: None }],
    }
}

pub(crate) struct PendingToolUse {
    pub message: MessageHandle,
    pub ordinal: u32,
    pub name: String,
    pub input: Value,
    pub started: Timestamp,
}

pub(crate) struct TranscriptState<'a> {
    pub root: &'a Path,
    pub handle: SourceFileHandle,
    pub session: SessionHandle,
    pub session_id: String,
    pub version_sent: bool,
    pub cwd_sent: bool,
    pub branch_sent: bool,
    pub models_sent: BTreeSet<String>,
    pub session_ids: BTreeSet<String>,
    pub origin_ids: BTreeSet<String>,
    pub pending: HashMap<String, PendingToolUse>,
}

impl<'a> TranscriptState<'a> {
    fn anomaly(&self, sink: &mut dyn ParseSink, kind: AnomalyKind, severity: Severity, offset: Option<u64>, message: String, details: Value) {
        sink.anomaly(AnomalyDraft {
            kind,
            severity,
            source_file: Some(self.handle),
            session: Some(self.session),
            byte_offset: offset,
            message,
            details,
        });
    }

    fn note_session_fields(&mut self, v: &Value, sink: &mut dyn ParseSink) {
        let mut update = SessionUpdate::default();
        if !self.version_sent {
            if let Some(ver) = str_field(v, "version") {
                update.harness_version = Some(ver);
                self.version_sent = true;
            }
        }
        if !self.cwd_sent {
            if let Some(cwd) = str_field(v, "cwd") {
                update.project_path = Some(cwd);
                self.cwd_sent = true;
            }
        }
        if !self.branch_sent {
            if let Some(b) = str_field(v, "gitBranch") {
                update.git_branch = Some(b);
                self.branch_sent = true;
            }
        }
        if let Some(sid) = str_field(v, "sessionId") {
            self.session_ids.insert(sid);
        }
        if let Some(sid) = str_field(v, "session_id") {
            self.origin_ids.insert(sid);
        }
        if update != SessionUpdate::default() {
            sink.update_session(self.session, update);
        }
    }

    fn timestamp_of(&self, v: &Value, prov: &Provenance, conversation: bool, sink: &mut dyn ParseSink) -> Timestamp {
        let raw = v.get("timestamp").and_then(Value::as_str);
        let ts = raw.and_then(Timestamp::stored);
        match ts {
            Some(t) => t,
            None => {
                if conversation {
                    self.anomaly(
                        sink,
                        AnomalyKind::MissingTimestamp,
                        Severity::Warning,
                        Some(prov.byte_offset),
                        match raw {
                            Some(r) => format!("conversation record has unparseable timestamp {r:?}"),
                            None => "conversation record has no timestamp".to_string(),
                        },
                        json!({ "record_type": v.get("type") }),
                    );
                }
                Timestamp::absent()
            }
        }
    }

    pub fn record(&mut self, v: Value, prov: Provenance, sink: &mut dyn ParseSink) {
        let rtype = v.get("type").and_then(Value::as_str).unwrap_or("").to_string();
        self.note_session_fields(&v, sink);
        match rtype.as_str() {
            "user" | "assistant" | "system" => self.conversation_record(&rtype, v, prov, sink),
            "ai-title" | "custom-title" => {
                let title = str_field(&v, "aiTitle").or_else(|| str_field(&v, "customTitle")).or_else(|| str_field(&v, "title"));
                if title.is_some() {
                    sink.update_session(self.session, SessionUpdate { title, ..Default::default() });
                }
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
            "summary" => {
                let title = str_field(&v, "summary");
                if title.is_some() {
                    sink.update_session(self.session, SessionUpdate { title, ..Default::default() });
                }
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
            "attachment" => {
                let blocks = vec![BlockDraft {
                    kind: BlockKind::Attachment,
                    text: v.get("attachment").and_then(|a| a.get("content")).and_then(Value::as_str).map(str::to_string),
                    payload: v.get("attachment").cloned().unwrap_or(Value::Null),
                    tool_use_id: None,
                }];
                self.meta_record(&rtype, v, prov, blocks, sink);
            }
            // FILE HISTORY (Task 9): "file-history-delta" gains an observation here.
            t if KNOWN_META.contains(&t) => self.meta_record(&rtype, v, prov, Vec::new(), sink),
            other => {
                self.anomaly(
                    sink,
                    AnomalyKind::UnknownRecordType,
                    Severity::Info,
                    Some(prov.byte_offset),
                    format!("unknown record type {other:?} kept as meta message"),
                    json!({ "record_type": other }),
                );
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
        }
    }

    fn attributes_of(v: &Value) -> Map<String, Value> {
        let mut attrs = Map::new();
        if let Value::Object(obj) = v {
            for (k, val) in obj {
                if k != "message" && k != "toolUseResult" {
                    attrs.insert(k.clone(), val.clone());
                }
            }
        }
        attrs
    }

    fn meta_record(&mut self, rtype: &str, v: Value, prov: Provenance, blocks: Vec<BlockDraft>, sink: &mut dyn ParseSink) {
        let timestamp = self.timestamp_of(&v, &prov, false, sink);
        let attributes = match &v {
            Value::Object(obj) => obj.clone(),
            other => {
                let mut m = Map::new();
                m.insert("value".to_string(), other.clone());
                m
            }
        };
        sink.message(
            self.session,
            MessageDraft {
                harness_record_type: rtype.to_string(),
                harness_uuid: str_field(&v, "uuid"),
                parent_uuid: str_field(&v, "parentUuid"),
                role: Role::Meta,
                timestamp,
                model: None,
                attributes,
                blocks,
                provenance: prov,
            },
        );
    }

    fn conversation_record(&mut self, rtype: &str, v: Value, prov: Provenance, sink: &mut dyn ParseSink) {
        let msg = v.get("message");
        let blocks = blocks_from_content(msg.and_then(|m| m.get("content")));
        let only_tool_results = !blocks.is_empty() && blocks.iter().all(|b| b.kind == BlockKind::ToolResult);
        let role = match (rtype, only_tool_results) {
            ("user", true) => Role::Tool,
            ("user", false) => Role::User,
            ("assistant", _) => Role::Assistant,
            _ => Role::System,
        };
        let timestamp = self.timestamp_of(&v, &prov, true, sink);
        let model = msg.and_then(|m| m.get("model")).and_then(Value::as_str).map(str::to_string);
        if let Some(m) = &model {
            if self.models_sent.insert(m.clone()) {
                sink.update_session(self.session, SessionUpdate { model: Some(m.clone()), ..Default::default() });
            }
        }
        let tool_use_result = v.get("toolUseResult").cloned();
        let attributes = Self::attributes_of(&v);
        let handle = sink.message(
            self.session,
            MessageDraft {
                harness_record_type: rtype.to_string(),
                harness_uuid: str_field(&v, "uuid"),
                parent_uuid: str_field(&v, "parentUuid"),
                role,
                timestamp: timestamp.clone(),
                model,
                attributes,
                blocks: blocks.clone(),
                provenance: prov.clone(),
            },
        );
        // TOOL PAIRING (Task 7): tool_use / tool_result blocks are paired here.
        let _ = (handle, tool_use_result, timestamp, prov);
    }

    pub fn finish(&mut self, sink: &mut dyn ParseSink) {
        // IDENTITY CLAIMS (Task 8) and UNFINISHED TOOL USES (Task 7) are emitted here.
    }
}

pub fn parse_transcript(ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
    let path = classify_path(ctx.rel_path)
        .ok_or_else(|| ParseError::Invalid(format!("not a transcript path: {}", ctx.rel_path.display())))?;
    let session = sink.session(SessionDraft {
        harness_session_id: path.session_id.clone(),
        kind: if path.is_subagent { SessionKind::Subagent } else { SessionKind::Primary },
        parent_harness_session_id: path.parent_session_id.clone(),
        title: None,
        project_path: None,
        git_branch: None,
        harness_version: None,
        first_ts: None,
        last_ts: None,
    });
    let mut state = TranscriptState {
        root: ctx.root,
        handle: ctx.handle,
        session,
        session_id: path.session_id.clone(),
        version_sent: false,
        cwd_sent: false,
        branch_sent: false,
        models_sent: BTreeSet::new(),
        session_ids: BTreeSet::new(),
        origin_ids: BTreeSet::new(),
        pending: HashMap::new(),
    };
    // FILE FLAGS (Task 8): orphaned / superseded anomalies are emitted here.
    let _ = &path.flags;
    let file = File::open(&ctx.abs_path)?;
    let reader = JsonlReader::new(BufReader::new(file));
    for rec in reader {
        let rec = rec?;
        let prov = provenance(ctx.handle, &rec);
        if rec.oversized {
            state.anomaly(
                sink,
                AnomalyKind::OversizedRecord,
                Severity::Warning,
                Some(rec.offset),
                format!("record of {} bytes exceeds the size cap and was skipped", rec.length),
                json!({ "length": rec.length }),
            );
            continue;
        }
        match serde_json::from_slice::<Value>(&rec.bytes) {
            Ok(v) => state.record(v, prov, sink),
            Err(e) => {
                let (kind, severity, msg) = if rec.terminated {
                    (AnomalyKind::MalformedRecord, Severity::Error, format!("line {} is not valid JSON: {e}", rec.index))
                } else {
                    (AnomalyKind::TruncatedLine, Severity::Warning, format!("final line {} is truncated (no newline, not valid JSON): {e}", rec.index))
                };
                state.anomaly(sink, kind, severity, Some(rec.offset), msg, json!({ "length": rec.length, "terminated": rec.terminated }));
            }
        }
    }
    state.finish(sink);
    Ok(())
}
```

Wire it into `crates/vem-adapters/src/claude_code/mod.rs`: add `pub mod transcript;` and replace `parse_file`:

```rust
    fn parse_file(&self, ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
        match ctx.store.kind.as_str() {
            STORE_PROJECTS => {
                if transcript::classify_path(ctx.rel_path).is_some() {
                    transcript::parse_transcript(ctx, sink)
                } else {
                    Ok(()) // meta.json and tool-results/*.txt are inventoried and retained, not parsed
                }
            }
            _ => Ok(()),
        }
    }
```

- [ ] **Step 4: Run the tests, accept the snapshot after reading it**

Run:
```bash
cargo test -p vem-adapters --test claude_code_transcript
```
Expected: 9 pass, `snapshot_of_all_messages` fails because no snapshot exists yet. Then:
```bash
INSTA_UPDATE=always cargo test -p vem-adapters --test claude_code_transcript snapshot_of_all_messages
cat crates/vem-adapters/tests/snapshots/claude_code_transcript__s1_messages.snap | head -80
```
Read the snapshot: 16 messages, in file order, roles as the first test asserts, every `provenance.content_sha256` a 64-hex string. Then run the full test file again; expected 10 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/vem-adapters
git commit -m "feat(adapters): claude code transcript parser with provenance and anomalies

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: Tool-call pairing and observations

**Files:**
- Create: `crates/vem-adapters/src/claude_code/tools.rs`
- Create: `crates/vem-adapters/tests/claude_code_tools.rs`
- Modify: `crates/vem-adapters/src/claude_code/transcript.rs` (the two `Task 7` markers)
- Modify: `crates/vem-adapters/src/claude_code/mod.rs` (`pub mod tools;`)

**Interfaces:**
- Consumes: `TranscriptState`, `PendingToolUse`, `str_field`, `flatten_text` from Task 6; `ParseSink::blob`.
- Produces: `tools::categorize(name: &str) -> ToolCategory`; `tools::derive_observations(name: &str, input: &Value, result: Option<&Value>, tool_call: ToolCallHandle, at: &Timestamp, sink: &mut dyn ParseSink) -> Vec<ObservationDraft>`; crate-private `tools::pair_blocks(state: &mut TranscriptState, message: MessageHandle, blocks: &[BlockDraft], tool_use_result: Option<&Value>, at: &Timestamp, prov: &Provenance, sink: &mut dyn ParseSink)`; `tools::flush_unfinished(state: &mut TranscriptState, sink)`.

- [ ] **Step 1: Write the failing tests**

`crates/vem-adapters/tests/claude_code_tools.rs`:

```rust
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-adapters --test claude_code_tools`
Expected: compile error (`tools` module missing) or assertion failures on `tool_calls.len()`.

- [ ] **Step 3: Implement `tools.rs`**

`crates/vem-adapters/src/claude_code/tools.rs`:

```rust
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
    pending.sort_by(|a, b| (a.1.message.0, a.1.ordinal).cmp(&(b.1.message.0, b.1.ordinal)));
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
```

- [ ] **Step 4: Wire pairing into the transcript parser**

In `crates/vem-adapters/src/claude_code/mod.rs` add `pub mod tools;`.

In `transcript.rs`, replace the two lines

```rust
        // TOOL PAIRING (Task 7): tool_use / tool_result blocks are paired here.
        let _ = (handle, tool_use_result, timestamp, prov);
```
with
```rust
        super::tools::pair_blocks(self, handle, &blocks, tool_use_result.as_ref(), &timestamp, &prov, sink);
```

and in `finish` replace the comment with:

```rust
        super::tools::flush_unfinished(self, sink);
        // IDENTITY CLAIMS (Task 8) are emitted here.
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p vem-adapters`
Expected: all pass, including Task 6's suite (the snapshot does not change: tool calls and observations are not part of `sink.messages`).

- [ ] **Step 6: Commit**

```bash
git add crates/vem-adapters
git commit -m "feat(adapters): pair claude code tool calls and derive command, file, web and subagent observations

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Subagent sessions, identity claims, orphaned and superseded files

**Files:**
- Modify: `crates/vem-adapters/src/claude_code/transcript.rs` (the `Task 8` markers)
- Create: `crates/vem-adapters/tests/claude_code_sessions.rs`

**Interfaces:**
- Consumes: `TranscriptState::{session_ids, origin_ids, session_id, handle}`, `classify_path(...).flags`.
- Produces identity-claim schemes the case layer (Task 12) resolves: `claude:sessionId` (every distinct `sessionId` seen; `matched` when equal to the file's own id, else `unmatched`), `claude:origin_session_id` (every distinct `session_id` differing from the file's id; `unmatched`), `claude:spawning_tool_use_id` (from a subagent's `.meta.json` `toolUseId`; `unmatched`).

- [ ] **Step 1: Write the failing tests**

`crates/vem-adapters/tests/claude_code_sessions.rs`:

```rust
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-adapters --test claude_code_sessions`
Expected: failures on the spawn claim, the sessionId claims and the orphaned anomaly; `classify_path_handles_all_shapes` passes already.

- [ ] **Step 3: Implement file flags, subagent meta and claims**

In `transcript.rs`, replace

```rust
    // FILE FLAGS (Task 8): orphaned / superseded anomalies are emitted here.
    let _ = &path.flags;
```
with
```rust
    let file_name = ctx.rel_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    for flag in &path.flags {
        let (kind, what) = match *flag {
            "orphaned" => (AnomalyKind::OrphanedFile, "set aside as orphaned by the harness"),
            _ => (AnomalyKind::SupersededFile, "set aside as superseded by the harness"),
        };
        state.anomaly(sink, kind, Severity::Info, None, format!("transcript {file_name} was {what}; parsed anyway"), json!({ "file_name": file_name }));
    }
    if path.is_subagent {
        read_subagent_meta(&mut state, &ctx.abs_path, sink);
    }
```

Add this function after `parse_transcript`:

```rust
/// `agent-<id>.meta.json` next to a subagent transcript: title from `description`, claim on `toolUseId`.
fn read_subagent_meta(state: &mut TranscriptState<'_>, abs_path: &Path, sink: &mut dyn ParseSink) {
    let meta_path = abs_path.with_extension("meta.json");
    let Ok(bytes) = std::fs::read(&meta_path) else { return };
    let Ok(meta) = serde_json::from_slice::<Value>(&bytes) else {
        state.anomaly(sink, AnomalyKind::MalformedRecord, Severity::Warning, None, format!("subagent meta file {} is not valid JSON", meta_path.display()), json!({}));
        return;
    };
    let title = str_field(&meta, "description");
    if title.is_some() {
        sink.update_session(state.session, SessionUpdate { title, ..Default::default() });
    }
    if let Some(tool_use_id) = str_field(&meta, "toolUseId") {
        sink.identity_claim(
            state.session,
            IdentityClaimDraft { scheme: "claude:spawning_tool_use_id".to_string(), claimed_id: tool_use_id, source_file: state.handle, join_status: JoinStatus::Unmatched },
        );
    }
    let mut attrs = Map::new();
    attrs.insert("subagent_meta".to_string(), meta);
    // The meta file content is kept on the session through a meta message with inferred provenance.
    sink.message(
        state.session,
        MessageDraft {
            harness_record_type: "subagent-meta".to_string(),
            harness_uuid: None,
            parent_uuid: None,
            role: Role::Meta,
            timestamp: Timestamp::absent(),
            model: None,
            attributes: attrs,
            blocks: Vec::new(),
            provenance: Provenance {
                source_file: state.handle,
                byte_offset: 0,
                byte_length: 0,
                record_index: 0,
                content_sha256: sha256_hex(&bytes),
                parser_name: PARSER_NAME.to_string(),
                parser_version: PARSER_VERSION.to_string(),
                origin: ProvOrigin::Derived,
            },
        },
    );
}
```

Note: `with_extension("meta.json")` on `agent-x.jsonl` yields `agent-x.meta.json`, which is the sibling's actual name.

Then in `finish`, replace `// IDENTITY CLAIMS (Task 8) are emitted here.` with:

```rust
        for sid in std::mem::take(&mut self.session_ids) {
            let join_status = if sid == self.session_id { JoinStatus::Matched } else { JoinStatus::Unmatched };
            sink.identity_claim(self.session, IdentityClaimDraft { scheme: "claude:sessionId".to_string(), claimed_id: sid, source_file: self.handle, join_status });
        }
        for sid in std::mem::take(&mut self.origin_ids) {
            if sid == self.session_id {
                continue;
            }
            sink.identity_claim(self.session, IdentityClaimDraft { scheme: "claude:origin_session_id".to_string(), claimed_id: sid, source_file: self.handle, join_status: JoinStatus::Unmatched });
        }
```

For subagent files the `sessionId` in records is the parent's id, so the `claude:sessionId` claim is `unmatched` there; the case layer matches it to the parent (Task 12).

- [ ] **Step 4: Run all adapter tests**

Run: `cargo test -p vem-adapters`
Expected: all pass. The Task 6 snapshot is unchanged because S1 is not a subagent file and the `subagent-meta` message appears only for subagents.

- [ ] **Step 5: Commit**

```bash
git add crates/vem-adapters
git commit -m "feat(adapters): subagent sessions, identity claims and orphaned/superseded flags for claude code

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Sidecars: file-history backups and history.jsonl

**Files:**
- Create: `crates/vem-adapters/src/claude_code/sidecars.rs`
- Create: `crates/vem-adapters/tests/claude_code_sidecars.rs`
- Modify: `crates/vem-adapters/src/claude_code/transcript.rs` (the `Task 9` marker)
- Modify: `crates/vem-adapters/src/claude_code/mod.rs` (`pub mod sidecars;` and the `STORE_HISTORY` dispatch)

**Interfaces:**
- Consumes: `TranscriptState`, `ParseSink::{find_session, blob, observation, anomaly}`.
- Produces: `sidecars::parse_history(ctx, sink) -> Result<(), ParseError>` (`history.jsonl`); `transcript::TranscriptState::file_history_delta(&mut self, v: &Value, prov: &Provenance, sink)`; parser name `claude_code.history` version `1`.

- [ ] **Step 1: Write the failing tests**

`crates/vem-adapters/tests/claude_code_sidecars.rs`:

```rust
mod common;

use common::*;
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::testing::VecSink;

#[test]
fn file_history_delta_yields_backup_observation_with_before_content() {
    let sink = parse_fixture(S1_FILE);
    let fh: Vec<&ObservationDraft> = sink
        .observations_of(ObservationKind::FileEdited)
        .into_iter()
        .filter(|o| o.details["source"] == "file-history-delta")
        .collect();
    assert_eq!(fh.len(), 1);
    assert_eq!(fh[0].path.as_deref(), Some("/home/alice/proj/notes.md"));
    assert_eq!(fh[0].confidence, Confidence::Medium);
    assert_eq!(fh[0].before_blob.as_deref(), Some(sha256_hex(b"# Notes\n").as_str()));
    assert_eq!(fh[0].details["backupFileName"], "deadbeef00000001@v1");
    assert_eq!(fh[0].timestamp.value.as_deref(), Some("2026-09-30T10:00:04.500Z"));
    assert!(matches!(fh[0].derived_from, Derivation::Record(_)));
}

#[test]
fn history_jsonl_yields_pastes_and_missing_transcripts() {
    let mut sink = VecSink::default();
    parse_file_into(&fixture_root(), S1_FILE, &mut sink);
    parse_file_into(&fixture_root(), "history.jsonl", &mut sink);
    let pastes = sink.observations_of(ObservationKind::PasteDetected);
    assert_eq!(pastes.len(), 1);
    assert_eq!(pastes[0].timestamp.value.as_deref(), Some("2026-10-01T10:01:00.000Z"));
    assert_eq!(pastes[0].details["pastedContents"]["1"]["content"], "AKIAIOSFODNN7EXAMPLE\nline2\nline3");
    assert_eq!(pastes[0].details["display"], "[Pasted text #1 +3 lines] please review");
    let missing = sink.anomalies_of(AnomalyKind::MissingTranscript);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].severity, Severity::Warning);
    assert_eq!(missing[0].details["sessionId"], "33333333-0000-4000-8000-000000000003");
    assert_eq!(missing[0].details["project"], "/home/alice/other");
}

#[test]
fn history_timestamps_of_odd_types_do_not_lose_the_entry() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("projects/-p")).unwrap();
    std::fs::write(
        tmp.path().join("projects/-p/abababab-0000-4000-8000-0000000000ab.jsonl"),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"x\"},\"uuid\":\"h1\",\"timestamp\":\"2026-09-30T10:00:00Z\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\",\"parentUuid\":null}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("history.jsonl"),
        concat!(
            "{\"display\":\"a\",\"pastedContents\":{\"1\":{\"content\":\"p\"}},\"timestamp\":\"1790848800000\",\"project\":\"/p\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\"}\n",
            "{\"display\":\"b\",\"pastedContents\":{\"1\":{\"content\":\"q\"}},\"timestamp\":1790848800000.5,\"project\":\"/p\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\"}\n",
            "{\"display\":\"c\",\"pastedContents\":{\"1\":{\"content\":\"r\"}},\"timestamp\":\"soon\",\"project\":\"/p\",\"sessionId\":\"abababab-0000-4000-8000-0000000000ab\"}\n",
        ),
    )
    .unwrap();
    let mut sink = VecSink::default();
    parse_file_into(tmp.path(), "projects/-p/abababab-0000-4000-8000-0000000000ab.jsonl", &mut sink);
    parse_file_into(tmp.path(), "history.jsonl", &mut sink);
    let pastes = sink.observations_of(ObservationKind::PasteDetected);
    assert_eq!(pastes.len(), 3);
    assert_eq!(pastes[0].timestamp.value.as_deref(), Some("2026-10-01T10:00:00.000Z"));
    assert_eq!(pastes[1].timestamp.value.as_deref(), Some("2026-10-01T10:00:00.000Z"));
    assert_eq!(pastes[2].timestamp, Timestamp::absent());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-adapters --test claude_code_sidecars`
Expected: the first test fails on `fh.len()` (0), the others fail on paste counts.

- [ ] **Step 3: Implement the file-history delta observation**

In `transcript.rs` replace the line

```rust
            // FILE HISTORY (Task 9): "file-history-delta" gains an observation here.
```
with
```rust
            "file-history-delta" => {
                self.file_history_delta(&v, &prov, sink);
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
```

and add to `impl TranscriptState`:

```rust
    /// A `file-history-delta` records that the harness backed up `trackingPath` before changing it.
    /// The backup lives at `file-history/<session>/<backupFileName>`; when present it is the before-content.
    pub fn file_history_delta(&mut self, v: &Value, prov: &Provenance, sink: &mut dyn ParseSink) {
        let tracking = str_field(v, "trackingPath");
        let backup = v.get("backup").cloned().unwrap_or(Value::Null);
        let backup_name = str_field(&backup, "backupFileName");
        let real_parent = str_field(&backup, "realParentDir");
        let path = match (&real_parent, &tracking) {
            (Some(dir), Some(t)) => {
                let base = Path::new(t).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| t.clone());
                Some(format!("{}/{}", dir.trim_end_matches('/'), base))
            }
            (None, Some(t)) => Some(t.clone()),
            _ => None,
        };
        let before_blob = backup_name.as_ref().and_then(|name| {
            let p = self.root.join("file-history").join(&self.session_id).join(name);
            std::fs::read(p).ok().map(|bytes| sink.blob(&bytes))
        });
        let timestamp = v
            .get("timestamp")
            .and_then(Value::as_str)
            .or_else(|| backup.get("backupTime").and_then(Value::as_str))
            .and_then(Timestamp::stored)
            .unwrap_or_else(Timestamp::absent);
        sink.observation(
            self.session,
            ObservationDraft {
                kind: ObservationKind::FileEdited,
                derived_from: Derivation::Record(prov.clone()),
                path,
                command: None,
                before_blob,
                after_blob: None,
                timestamp,
                confidence: Confidence::Medium,
                details: json!({
                    "source": "file-history-delta",
                    "trackingPath": tracking,
                    "backupFileName": backup_name,
                    "version": backup.get("version").cloned().unwrap_or(Value::Null),
                    "backupTime": backup.get("backupTime").cloned().unwrap_or(Value::Null),
                    "messageId": str_field(v, "messageId"),
                }),
            },
        );
    }
```

A `before_blob` of `None` while `backupFileName` is set tells the examiner the backup file itself is missing from `file-history/`.

- [ ] **Step 4: Implement the history.jsonl parser**

`crates/vem-adapters/src/claude_code/sidecars.rs`:

```rust
//! Sidecar stores of a `.claude` directory: `history.jsonl` (prompt history with pasted content).

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufReader;
use vem_core::adapter::{FileContext, ParseError};
use vem_core::hash::sha256_hex;
use vem_core::jsonl::JsonlReader;
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const HISTORY_PARSER_NAME: &str = "claude_code.history";
pub const HISTORY_PARSER_VERSION: &str = "1";

fn epoch_ms(v: Option<&Value>) -> Timestamp {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).and_then(Timestamp::stored_epoch_ms),
        Some(Value::String(s)) => s.parse::<i64>().ok().and_then(Timestamp::stored_epoch_ms).or_else(|| Timestamp::stored(s)),
        _ => None,
    }
    .unwrap_or_else(Timestamp::absent)
}

/// Each `history.jsonl` line is one submitted prompt: `display`, `pastedContents`, `timestamp` (ms), `project`, `sessionId`.
/// Pasted content becomes a `paste_detected` observation; a session id with no transcript is a `missing_transcript` anomaly.
pub fn parse_history(ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
    let file = File::open(&ctx.abs_path)?;
    let mut missing: BTreeSet<String> = BTreeSet::new();
    for rec in JsonlReader::new(BufReader::new(file)) {
        let rec = rec?;
        let prov = Provenance {
            source_file: ctx.handle,
            byte_offset: rec.offset,
            byte_length: rec.length,
            record_index: rec.index,
            content_sha256: sha256_hex(&rec.bytes),
            parser_name: HISTORY_PARSER_NAME.to_string(),
            parser_version: HISTORY_PARSER_VERSION.to_string(),
            origin: ProvOrigin::Stored,
        };
        let v: Value = match serde_json::from_slice(&rec.bytes) {
            Ok(v) => v,
            Err(e) => {
                sink.anomaly(AnomalyDraft {
                    kind: if rec.terminated { AnomalyKind::MalformedRecord } else { AnomalyKind::TruncatedLine },
                    severity: if rec.terminated { Severity::Error } else { Severity::Warning },
                    source_file: Some(ctx.handle),
                    session: None,
                    byte_offset: Some(rec.offset),
                    message: format!("history.jsonl line {} is not valid JSON: {e}", rec.index),
                    details: json!({}),
                });
                continue;
            }
        };
        let session_id = v.get("sessionId").and_then(Value::as_str).unwrap_or("").to_string();
        let timestamp = epoch_ms(v.get("timestamp"));
        let pasted = v.get("pastedContents").and_then(Value::as_object).map(|m| !m.is_empty()).unwrap_or(false);
        match sink.find_session(&session_id) {
            Some(session) => {
                if pasted {
                    sink.observation(
                        session,
                        ObservationDraft {
                            kind: ObservationKind::PasteDetected,
                            derived_from: Derivation::Record(prov),
                            path: None,
                            command: None,
                            before_blob: None,
                            after_blob: None,
                            timestamp,
                            confidence: Confidence::High,
                            details: json!({
                                "display": v.get("display").cloned().unwrap_or(Value::Null),
                                "pastedContents": v.get("pastedContents").cloned().unwrap_or(Value::Null),
                                "project": v.get("project").cloned().unwrap_or(Value::Null),
                            }),
                        },
                    );
                }
            }
            None => {
                if !session_id.is_empty() && missing.insert(session_id.clone()) {
                    sink.anomaly(AnomalyDraft {
                        kind: AnomalyKind::MissingTranscript,
                        severity: Severity::Warning,
                        source_file: Some(ctx.handle),
                        session: None,
                        byte_offset: Some(rec.offset),
                        message: format!("history.jsonl references session {session_id} but no transcript exists in this root"),
                        details: json!({
                            "sessionId": session_id,
                            "project": v.get("project").cloned().unwrap_or(Value::Null),
                            "display": v.get("display").cloned().unwrap_or(Value::Null),
                            "timestamp": timestamp.value,
                        }),
                    });
                }
            }
        }
    }
    Ok(())
}
```

In `crates/vem-adapters/src/claude_code/mod.rs` add `pub mod sidecars;` and the dispatch arm `STORE_HISTORY => sidecars::parse_history(ctx, sink),` before the `_ => Ok(())` arm.

- [ ] **Step 5: Run all adapter tests and refresh the snapshot**

Run: `cargo test -p vem-adapters`
Expected: the Task 6 snapshot test fails because nothing in `messages` changed... verify: `file-history-delta` still yields the same meta message, so the snapshot is unchanged and all tests pass. If the snapshot does fail, inspect the diff with `cargo insta test -p vem-adapters --review` or `INSTA_UPDATE=always`, confirm only expected differences, and re-run.

- [ ] **Step 6: Commit**

```bash
git add crates/vem-adapters
git commit -m "feat(adapters): claude code file-history backups and history.jsonl pastes and missing transcripts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Case directory, schema, blobs and append-only audit log

**Files:**
- Create: `crates/vem-case/src/error.rs`
- Create: `crates/vem-case/src/db.rs`
- Create: `crates/vem-case/src/schema.sql`
- Create: `crates/vem-case/src/blobs.rs`
- Create: `crates/vem-case/src/case.rs`
- Create: `crates/vem-case/tests/case_db.rs`
- Modify: `crates/vem-case/src/lib.rs`

**Interfaces:**
- Produces: `vem_case::{Case, CaseError, TOOL_VERSION}`; `Case::create(dir: &Path, name: &str, examiner: Option<&str>) -> Result<Case, CaseError>`, `Case::open(dir) -> Result<Case, CaseError>`, `Case::info() -> Result<CaseInfo { name, examiner, created_at, tool_version }>`, `Case::audit(action: &str, target: Option<&str>, details: Value) -> Result<()>`, `Case::blob_path(sha) -> PathBuf`, `Case::put_blob(bytes) -> Result<String>`; `pub conn: rusqlite::Connection` and `pub dir: PathBuf` fields; `vem_case::blobs::{put_bytes(conn, dir, bytes) -> Result<String>, put_file(conn, dir, src, sha, size) -> Result<()>, path(dir, sha) -> PathBuf}`; `vem_case::case::now() -> String` (UTC in `TS_FORMAT`); `vem_case::db::{open(path) -> Result<Connection>, SCHEMA_VERSION}`.
- The schema in `schema.sql` is the one every later task writes to and reads from; column names below are authoritative.

- [ ] **Step 1: Write the failing tests**

`crates/vem-case/tests/case_db.rs`:

```rust
use vem_case::{Case, CaseError};

#[test]
fn create_then_open_round_trips_case_info() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("case1");
    let case = Case::create(&dir, "Incident 42", Some("examiner a")).unwrap();
    assert!(dir.join("case.db").is_file());
    assert!(dir.join("blobs").is_dir());
    assert!(dir.join("exports").is_dir());
    drop(case);
    let case = Case::open(&dir).unwrap();
    let info = case.info().unwrap();
    assert_eq!(info.name, "Incident 42");
    assert_eq!(info.examiner.as_deref(), Some("examiner a"));
    assert_eq!(info.tool_version, vem_case::TOOL_VERSION);
    let v: i64 = case.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, vem_case::db::SCHEMA_VERSION);
}

#[test]
fn refuses_non_empty_dir_and_non_case_dir() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("something.txt"), "x").unwrap();
    assert!(matches!(Case::create(tmp.path(), "n", None), Err(CaseError::AlreadyExists(_))));
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(Case::open(empty.path()), Err(CaseError::NotACase(_))));
}

#[test]
fn audit_log_is_append_only() {
    let tmp = tempfile::tempdir().unwrap();
    let case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    case.audit("test.action", Some("t"), serde_json::json!({"k": 1})).unwrap();
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 2, "case.create plus test.action");
    assert!(case.conn.execute("UPDATE audit_log SET action = 'x'", []).is_err());
    assert!(case.conn.execute("DELETE FROM audit_log", []).is_err());
    let still: i64 = case.conn.query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0)).unwrap();
    assert_eq!(still, 2);
}

#[test]
fn blobs_are_content_addressed_and_deduplicated() {
    let tmp = tempfile::tempdir().unwrap();
    let case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let a = case.put_blob(b"hello").unwrap();
    let b = case.put_blob(b"hello").unwrap();
    assert_eq!(a, b);
    assert_eq!(a, vem_core::hash::sha256_hex(b"hello"));
    assert_eq!(std::fs::read(case.blob_path(&a)).unwrap(), b"hello");
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM blobs", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-case --test case_db`
Expected: compile errors (`Case` not found).

- [ ] **Step 3: Write the error type, schema and db module**

`crates/vem-case/src/error.rs`:

```rust
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CaseError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("directory is not empty, refusing to create a case in it: {0}")]
    AlreadyExists(PathBuf),
    #[error("not a case directory (no case.db): {0}")]
    NotACase(PathBuf),
    #[error("case schema version {0} is newer than this tool supports")]
    SchemaTooNew(i64),
    #[error("evidence path is not recognized as a harness directory: {path}{hint}")]
    Unrecognized { path: PathBuf, hint: String },
    #[error("evidence path matches several harnesses ({0}); pass --harness to choose")]
    Ambiguous(String),
    #[error("no evidence root with id {0}")]
    NoSuchRoot(i64),
    #[error("no adapter for harness {0}")]
    NoAdapter(String),
    #[error("export: {0}")]
    Export(String),
}
```

`crates/vem-case/src/schema.sql`:

```sql
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

CREATE TABLE source_files (
    id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES evidence_roots(id),
    store_id INTEGER REFERENCES stores(id),
    rel_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    mtime TEXT,
    ctime TEXT,
    atime TEXT,
    retained INTEGER NOT NULL DEFAULT 0,
    parse_status TEXT NOT NULL DEFAULT 'unparsed',
    parse_error TEXT,
    record_count INTEGER NOT NULL DEFAULT 0,
    anomaly_count INTEGER NOT NULL DEFAULT 0,
    ingested_at TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    UNIQUE (root_id, rel_path, version)
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
```

`crates/vem-case/src/db.rs`:

```rust
//! SQLite connection setup and schema migration (user_version based).

use crate::error::CaseError;
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i64 = 1;

pub fn open(path: &Path) -> Result<Connection, CaseError> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<(), CaseError> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(CaseError::SchemaTooNew(version));
    }
    if version < 1 {
        conn.execute_batch(include_str!("schema.sql"))?;
        conn.execute_batch("PRAGMA user_version = 1;")?;
    }
    Ok(())
}
```

- [ ] **Step 4: Write blobs and case modules**

`crates/vem-case/src/blobs.rs`:

```rust
//! Content-addressed blob store: `<case>/blobs/<sha256>` plus a `blobs` index table.
//! Free functions take a `&Connection` so they work inside a transaction.

use crate::error::CaseError;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use vem_core::hash::sha256_hex;

pub fn path(case_dir: &Path, sha256: &str) -> PathBuf {
    case_dir.join("blobs").join(sha256)
}

fn record(conn: &Connection, sha256: &str, size: u64) -> Result<(), CaseError> {
    conn.execute("INSERT OR IGNORE INTO blobs (sha256, size) VALUES (?1, ?2)", params![sha256, size as i64])?;
    Ok(())
}

fn write_atomically(target: &Path, write: impl FnOnce(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
    if target.exists() {
        return Ok(());
    }
    let tmp = target.with_extension(format!("tmp.{}", std::process::id()));
    write(&tmp)?;
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            if target.exists() { Ok(()) } else { Err(e) }
        }
    }
}

pub fn put_bytes(conn: &Connection, case_dir: &Path, bytes: &[u8]) -> Result<String, CaseError> {
    let sha = sha256_hex(bytes);
    write_atomically(&path(case_dir, &sha), |tmp| std::fs::write(tmp, bytes))?;
    record(conn, &sha, bytes.len() as u64)?;
    Ok(sha)
}

/// Copies `src` (already hashed as `sha256`, `size` bytes) into the store.
pub fn put_file(conn: &Connection, case_dir: &Path, src: &Path, sha256: &str, size: u64) -> Result<(), CaseError> {
    write_atomically(&path(case_dir, sha256), |tmp| std::fs::copy(src, tmp).map(|_| ()))?;
    record(conn, sha256, size)?;
    Ok(())
}
```

`crates/vem-case/src/case.rs`:

```rust
//! A case: one directory holding `case.db`, `blobs/` and `exports/`.

use crate::error::CaseError;
use crate::{blobs, db, TOOL_VERSION};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use vem_core::model::TS_FORMAT;

pub fn now() -> String {
    chrono::Utc::now().format(TS_FORMAT).to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseInfo {
    pub name: String,
    pub examiner: Option<String>,
    pub created_at: String,
    pub tool_version: String,
}

pub struct Case {
    pub dir: PathBuf,
    pub conn: Connection,
}

impl Case {
    pub fn create(dir: &Path, name: &str, examiner: Option<&str>) -> Result<Case, CaseError> {
        if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
            return Err(CaseError::AlreadyExists(dir.to_path_buf()));
        }
        std::fs::create_dir_all(dir.join("blobs"))?;
        std::fs::create_dir_all(dir.join("exports"))?;
        let conn = db::open(&dir.join("case.db"))?;
        conn.execute(
            "INSERT INTO case_info (id, name, examiner, created_at, tool_version) VALUES (1, ?1, ?2, ?3, ?4)",
            params![name, examiner, now(), TOOL_VERSION],
        )?;
        let case = Case { dir: dir.to_path_buf(), conn };
        case.audit("case.create", Some(name), serde_json::json!({ "examiner": examiner, "tool_version": TOOL_VERSION }))?;
        Ok(case)
    }

    pub fn open(dir: &Path) -> Result<Case, CaseError> {
        let db_path = dir.join("case.db");
        if !db_path.is_file() {
            return Err(CaseError::NotACase(dir.to_path_buf()));
        }
        let conn = db::open(&db_path)?;
        Ok(Case { dir: dir.to_path_buf(), conn })
    }

    pub fn info(&self) -> Result<CaseInfo, CaseError> {
        Ok(self.conn.query_row(
            "SELECT name, examiner, created_at, tool_version FROM case_info WHERE id = 1",
            [],
            |r| Ok(CaseInfo { name: r.get(0)?, examiner: r.get(1)?, created_at: r.get(2)?, tool_version: r.get(3)? }),
        )?)
    }

    pub fn audit(&self, action: &str, target: Option<&str>, details: Value) -> Result<(), CaseError> {
        self.conn.execute(
            "INSERT INTO audit_log (ts, action, target, details) VALUES (?1, ?2, ?3, ?4)",
            params![now(), action, target, details.to_string()],
        )?;
        Ok(())
    }

    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        blobs::path(&self.dir, sha256)
    }

    pub fn put_blob(&self, bytes: &[u8]) -> Result<String, CaseError> {
        blobs::put_bytes(&self.conn, &self.dir, bytes)
    }

    pub fn has_blob(&self, sha256: &str) -> Result<bool, CaseError> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM blobs WHERE sha256 = ?1", params![sha256], |_| Ok(()))
            .optional()?
            .is_some())
    }
}
```

`crates/vem-case/src/lib.rs`:

```rust
//! Case directory, SQLite database, ingest pipeline, verification, queries and exports.

pub mod blobs;
pub mod case;
pub mod db;
pub mod error;

pub use case::{Case, CaseInfo};
pub use error::CaseError;

pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p vem-case --test case_db`
Expected: 4 passed.

- [ ] **Step 6: Commit**

```bash
git add crates/vem-case
git commit -m "feat(case): case directory, sqlite schema, blob store and append-only audit log

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Attach evidence: identify, manifest, retain

**Files:**
- Create: `crates/vem-case/src/evidence.rs`
- Create: `crates/vem-case/tests/common/mod.rs`
- Create: `crates/vem-case/tests/evidence.rs`
- Modify: `crates/vem-case/src/lib.rs`

**Interfaces:**
- Consumes: `vem_adapters::{identify_root, adapter_for}`, `vem_core::hash::sha256_file`, `blobs::put_file`.
- Produces: `vem_case::evidence::{AttachOptions { label, host, user, os, harness: Option<Harness>, retain: bool }, AttachReport { root_id, harness, evidence, stores: Vec<StoreSummary { id, kind, generation, rel_path, file_count }>, absent, file_count, unclaimed_files, total_bytes, unreadable: Vec<String> }, attach(case: &mut Case, path: &Path, opts: AttachOptions) -> Result<AttachReport, CaseError>, child_candidates(path: &Path) -> Vec<(PathBuf, Vec<Identification>)>, rel_string(path: &Path) -> String}`.
- Test helper `tests/common/mod.rs::{fixture_root(), tree_fingerprint(path) -> BTreeMap<String, (u64, String)>, copy_dir(src, dst)}` reused by Tasks 12, 13.

- [ ] **Step 1: Write the test helper and failing tests**

`crates/vem-case/tests/common/mod.rs`:

```rust
#![allow(dead_code)]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vem_core::hash::sha256_file;

pub fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/claude-code/basic").canonicalize().unwrap()
}

pub const S1: &str = "0f0f0f0f-0000-4000-8000-000000000001";
pub const S0: &str = "0f0f0f0f-0000-4000-8000-000000000000";

/// rel path -> (mtime nanos, sha256). Used to prove evidence was not touched.
pub fn tree_fingerprint(root: &Path) -> BTreeMap<String, (u128, String)> {
    let mut out = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(root).unwrap().to_string_lossy().to_string();
        let mtime = entry.metadata().unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let (sha, _) = sha256_file(entry.path()).unwrap();
        out.insert(rel, (mtime, sha));
    }
    out
}

pub fn copy_dir(src: &Path, dst: &Path) {
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

pub fn file_count(root: &Path) -> usize {
    walkdir::WalkDir::new(root).into_iter().filter(|e| e.as_ref().unwrap().file_type().is_file()).count()
}
```

`crates/vem-case/tests/evidence.rs`:

```rust
mod common;

use common::*;
use vem_case::evidence::{attach, child_candidates, AttachOptions};
use vem_case::{Case, CaseError};
use vem_core::hash::sha256_file;
use vem_core::model::Harness;

fn opts(label: &str) -> AttachOptions {
    AttachOptions { label: label.to_string(), host: Some("laptop-1".into()), user: Some("alice".into()), os: Some("linux".into()), harness: None, retain: true }
}

#[test]
fn attaches_fixture_with_manifest_and_retention() {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let before = tree_fingerprint(&fixture_root());
    let report = attach(&mut case, &fixture_root(), opts("alice .claude")).unwrap();
    assert_eq!(tree_fingerprint(&fixture_root()), before, "evidence must not be touched");
    assert_eq!(report.harness, Harness::ClaudeCode);
    assert_eq!(report.stores[0].kind, "claude:projects");
    assert_eq!(report.file_count, file_count(&fixture_root()));
    assert_eq!(report.unclaimed_files, 0);
    assert!(report.absent.contains(&"claude:todos".to_string()));
    assert!(report.unreadable.is_empty());

    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM source_files WHERE root_id = ?1", [report.root_id], |r| r.get(0)).unwrap();
    assert_eq!(n as usize, report.file_count);
    let mut stmt = case.conn.prepare("SELECT rel_path, sha256, size, retained, store_id FROM source_files WHERE root_id = ?1").unwrap();
    let rows: Vec<(String, String, i64, i64, Option<i64>)> = stmt
        .query_map([report.root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for (rel, sha, size, retained, store_id) in &rows {
        let (actual, actual_size) = sha256_file(&fixture_root().join(rel)).unwrap();
        assert_eq!(&actual, sha, "{rel}");
        assert_eq!(actual_size as i64, *size);
        assert_eq!(*retained, 1);
        assert!(case.blob_path(sha).is_file(), "retained copy of {rel}");
        assert!(store_id.is_some(), "{rel} should belong to a store");
    }
    assert!(rows.iter().any(|(rel, ..)| rel == "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl"));
    let (label, harness, ident): (String, String, String) = case
        .conn
        .query_row("SELECT label, harness, identification FROM evidence_roots WHERE id = ?1", [report.root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!(label, "alice .claude");
    assert_eq!(harness, "claude-code");
    assert!(ident.contains("parentUuid"));
    let audits: i64 = case.conn.query_row("SELECT COUNT(*) FROM audit_log WHERE action = 'evidence.attach'", [], |r| r.get(0)).unwrap();
    assert_eq!(audits, 1);
}

#[test]
fn unclaimed_files_are_manifested_without_a_store() {
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    std::fs::write(ev.join("random-note.txt"), "left behind").unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let report = attach(&mut case, &ev, opts("x")).unwrap();
    assert_eq!(report.unclaimed_files, 1);
    let store_id: Option<i64> = case.conn.query_row("SELECT store_id FROM source_files WHERE rel_path = 'random-note.txt'", [], |r| r.get(0)).unwrap();
    assert!(store_id.is_none());
}

#[test]
fn no_retain_skips_copies() {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let mut o = opts("x");
    o.retain = false;
    attach(&mut case, &fixture_root(), o).unwrap();
    let retained: i64 = case.conn.query_row("SELECT COUNT(*) FROM source_files WHERE retained = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(retained, 0);
    assert_eq!(std::fs::read_dir(case.dir.join("blobs")).unwrap().count(), 0);
}

#[test]
fn unrecognized_dir_fails_with_child_hint_and_forced_harness_works() {
    let tmp = tempfile::tempdir().unwrap();
    let collection = tmp.path().join("collection");
    copy_dir(&fixture_root(), &collection.join(".claude"));
    std::fs::create_dir_all(collection.join("unrelated")).unwrap();
    let children = child_candidates(&collection);
    assert_eq!(children.len(), 1);
    assert!(children[0].0.ends_with(".claude"));
    assert_eq!(children[0].1[0].harness, Harness::ClaudeCode);

    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    match attach(&mut case, &collection, opts("x")) {
        Err(CaseError::Unrecognized { hint, .. }) => assert!(hint.contains(".claude"), "{hint}"),
        other => panic!("expected Unrecognized, got {:?}", other.map(|_| ())),
    }
    let roots: i64 = case.conn.query_row("SELECT COUNT(*) FROM evidence_roots", [], |r| r.get(0)).unwrap();
    assert_eq!(roots, 0, "a failed attach leaves nothing behind");

    let mut forced = opts("forced");
    forced.harness = Some(Harness::ClaudeCode);
    let report = attach(&mut case, &collection.join("unrelated"), forced).unwrap();
    assert_eq!(report.harness, Harness::ClaudeCode);
    assert!(report.evidence.iter().any(|e| e.contains("forced")));
    assert_eq!(report.file_count, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-case --test evidence`
Expected: compile error, `evidence` module missing.

- [ ] **Step 3: Implement attach**

`crates/vem-case/src/evidence.rs`:

```rust
//! Attaching a collected harness directory: identify by content, discover stores, hash every file,
//! retain copies (spec §3, §5).

use crate::case::now;
use crate::error::CaseError;
use crate::{blobs, Case};
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use vem_core::adapter::Identification;
use vem_core::hash::sha256_file;
use vem_core::model::{format_system_time, Harness};

#[derive(Debug, Clone)]
pub struct AttachOptions {
    pub label: String,
    pub host: Option<String>,
    pub user: Option<String>,
    pub os: Option<String>,
    pub harness: Option<Harness>,
    pub retain: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreSummary {
    pub id: i64,
    pub kind: String,
    pub generation: Option<String>,
    pub rel_path: String,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachReport {
    pub root_id: i64,
    pub harness: Harness,
    pub evidence: Vec<String>,
    pub stores: Vec<StoreSummary>,
    pub absent: Vec<String>,
    pub file_count: usize,
    pub unclaimed_files: usize,
    pub total_bytes: u64,
    pub unreadable: Vec<String>,
}

/// Forward-slash relative path string, whatever the host separator.
pub fn rel_string(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/")
}

/// Immediate child directories of `path` that identify as a harness directory (one level only).
pub fn child_candidates(path: &Path) -> Vec<(PathBuf, Vec<Identification>)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(path) else { return out };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        let ids = vem_adapters::identify_root(&d);
        if !ids.is_empty() {
            out.push((d, ids));
        }
    }
    out
}

fn identify(root: &Path, forced: Option<Harness>) -> Result<Identification, CaseError> {
    let ids = vem_adapters::identify_root(root);
    match (forced, ids.len()) {
        (Some(h), _) => Ok(ids
            .into_iter()
            .find(|i| i.harness == h)
            .unwrap_or(Identification { harness: h, evidence: vec!["forced by --harness; no content signature matched".to_string()] })),
        (None, 1) => Ok(ids.into_iter().next().expect("one")),
        (None, 0) => {
            let children = child_candidates(root);
            let hint = if children.is_empty() {
                String::new()
            } else {
                let names: Vec<String> = children
                    .iter()
                    .map(|(p, ids)| format!("{} ({})", p.display(), ids.iter().map(|i| i.harness.as_str()).collect::<Vec<_>>().join(", ")))
                    .collect();
                format!("; it contains harness directories you can attach individually: {}", names.join("; "))
            };
            Err(CaseError::Unrecognized { path: root.to_path_buf(), hint })
        }
        (None, _) => Err(CaseError::Ambiguous(ids.iter().map(|i| i.harness.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

pub fn attach(case: &mut Case, path: &Path, opts: AttachOptions) -> Result<AttachReport, CaseError> {
    let root = path.canonicalize()?;
    let identification = identify(&root, opts.harness)?;
    let adapter = vem_adapters::adapter_for(identification.harness)
        .ok_or_else(|| CaseError::NoAdapter(identification.harness.to_string()))?;
    let discovery = adapter.discover(&root);
    let case_dir = case.dir.clone();
    let tx = case.conn.transaction()?;

    tx.execute(
        "INSERT INTO evidence_roots (path, label, host, user, os, harness, attached_at, identification) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            root.to_string_lossy().to_string(),
            opts.label,
            opts.host,
            opts.user,
            opts.os,
            identification.harness.as_str(),
            now(),
            serde_json::to_string(&identification.evidence)?,
        ],
    )?;
    let root_id = tx.last_insert_rowid();

    let mut file_to_store: HashMap<String, i64> = HashMap::new();
    let mut stores = Vec::new();
    for s in &discovery.stores {
        tx.execute(
            "INSERT INTO stores (root_id, harness, kind, generation, rel_path, discovery_method, status) VALUES (?1, ?2, ?3, ?4, ?5, 'signature', 'pending')",
            params![root_id, identification.harness.as_str(), s.kind, s.generation, rel_string(&s.rel_path)],
        )?;
        let store_id = tx.last_insert_rowid();
        for f in &s.files {
            file_to_store.insert(rel_string(f), store_id);
        }
        stores.push(StoreSummary { id: store_id, kind: s.kind.clone(), generation: s.generation.clone(), rel_path: rel_string(&s.rel_path), file_count: s.files.len() });
    }
    for kind in &discovery.absent {
        tx.execute("INSERT INTO absent_stores (root_id, kind) VALUES (?1, ?2)", params![root_id, kind])?;
    }

    let mut file_count = 0usize;
    let mut unclaimed = 0usize;
    let mut total_bytes = 0u64;
    let mut unreadable = Vec::new();
    for entry in walkdir::WalkDir::new(&root).follow_links(false).sort_by_file_name() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                unreadable.push(e.path().map(|p| p.display().to_string()).unwrap_or_default());
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = rel_string(entry.path().strip_prefix(&root).unwrap_or(entry.path()));
        let (sha, size) = match sha256_file(entry.path()) {
            Ok(x) => x,
            Err(_) => {
                unreadable.push(rel);
                continue;
            }
        };
        let meta = entry.metadata().ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok()).map(format_system_time);
        let ctime = meta.as_ref().and_then(|m| m.created().ok()).map(format_system_time);
        let atime = meta.as_ref().and_then(|m| m.accessed().ok()).map(format_system_time);
        let store_id = file_to_store.get(&rel).copied();
        if store_id.is_none() {
            unclaimed += 1;
        }
        let retained = if opts.retain {
            blobs::put_file(&tx, &case_dir, entry.path(), &sha, size)?;
            1
        } else {
            0
        };
        tx.execute(
            "INSERT INTO source_files (root_id, store_id, rel_path, size, sha256, mtime, ctime, atime, retained, parse_status, version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'unparsed', 1)",
            params![root_id, store_id, rel, size as i64, sha, mtime, ctime, atime, retained],
        )?;
        file_count += 1;
        total_bytes += size;
    }
    tx.commit()?;

    let report = AttachReport {
        root_id,
        harness: identification.harness,
        evidence: identification.evidence,
        stores,
        absent: discovery.absent,
        file_count,
        unclaimed_files: unclaimed,
        total_bytes,
        unreadable,
    };
    case.audit(
        "evidence.attach",
        Some(&root.to_string_lossy()),
        serde_json::json!({ "root_id": root_id, "harness": report.harness, "file_count": file_count, "total_bytes": total_bytes, "retained": opts.retain, "unreadable": report.unreadable }),
    )?;
    Ok(report)
}
```

Add `pub mod evidence;` to `crates/vem-case/src/lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p vem-case --test evidence`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/vem-case
git commit -m "feat(case): attach evidence roots with content identification, manifest and retention

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: Ingest pipeline: database sink, per-file parsing, session linking and finalization

**Files:**
- Create: `crates/vem-case/src/sink.rs`
- Create: `crates/vem-case/src/ingest.rs`
- Create: `crates/vem-case/src/link.rs`
- Create: `crates/vem-case/tests/ingest.rs`
- Modify: `crates/vem-case/src/lib.rs`

**Interfaces:**
- Consumes: `ParseSink`, `HarnessAdapter::parse_file`, `FileContext`, `StoreCandidate`, schema from Task 10.
- Produces: `vem_case::ingest::{ingest(case: &mut Case, root_filter: Option<i64>) -> Result<IngestReport, CaseError>, IngestReport { files_parsed, files_failed, files_skipped, sessions, messages, tool_calls, observations, anomalies }}`; `vem_case::sink::DbSink<'a>` (`new(conn: &'a Connection, case_dir: &'a Path, root_id, store_id, source_file_id)`, `counts: SinkCounts`); `vem_case::link::{link_sessions(conn, root_id), finalize_sessions(conn, root_id)}`.

- [ ] **Step 1: Write the failing tests**

`crates/vem-case/tests/ingest.rs`:

```rust
mod common;

use common::*;
use rusqlite::params;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::Case;

fn attached_case() -> (tempfile::TempDir, Case, i64) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    let r = attach(&mut case, &fixture_root(), AttachOptions { label: "l".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    (tmp, case, r.root_id)
}

fn count(case: &Case, sql: &str) -> i64 {
    case.conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn session_id(case: &Case, harness_id: &str) -> i64 {
    case.conn.query_row("SELECT id FROM sessions WHERE harness_session_id = ?1", [harness_id], |r| r.get(0)).unwrap()
}

#[test]
fn ingests_fixture_into_canonical_rows() {
    let (_tmp, mut case, _root) = attached_case();
    let before = tree_fingerprint(&fixture_root());
    let report = ingest(&mut case, None).unwrap();
    assert_eq!(tree_fingerprint(&fixture_root()), before, "evidence must not be touched");
    assert_eq!(report.files_failed, 0);
    assert_eq!(report.sessions, 4, "S0, S1, subagent, orphan");
    assert_eq!(report.messages, 22, "16 + 2 + (2 + subagent-meta) + 1");
    assert_eq!(report.tool_calls, 4);
    assert_eq!(report.observations, 6, "command, written, edited, spawned, file-history backup, paste");
    assert_eq!(report.anomalies, 4, "truncated line, unknown record type, orphaned file, missing transcript");
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 4);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM messages"), 22);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM blocks"), count(&case, "SELECT COUNT(*) FROM blocks_fts"));
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies WHERE kind = 'truncated_line'"), 1);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies WHERE kind = 'missing_transcript' AND session_id IS NULL"), 1);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM source_files WHERE parse_status = 'parsed'"), count(&case, "SELECT COUNT(*) FROM source_files"));
    assert_eq!(count(&case, "SELECT COUNT(*) FROM stores WHERE kind = 'claude:projects' AND status = 'parsed'"), 1);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM stores WHERE kind = 'claude:settings' AND status = 'inventoried'"), 1);
    let (records, anomalies): (i64, i64) = case
        .conn
        .query_row("SELECT record_count, anomaly_count FROM source_files WHERE rel_path LIKE '%000000000001.jsonl'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((records, anomalies), (16, 2));
}

#[test]
fn links_subagents_resumptions_and_claims() {
    let (_tmp, mut case, _root) = attached_case();
    ingest(&mut case, None).unwrap();
    let s1 = session_id(&case, S1);
    let s0 = session_id(&case, S0);
    let sub = session_id(&case, "agent-0123456789abcdef");
    let (kind, parent): (String, Option<i64>) = case.conn.query_row("SELECT kind, parent_session_id FROM sessions WHERE id = ?1", [sub], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(kind, "subagent");
    assert_eq!(parent, Some(s1));
    let s1_kind: String = case.conn.query_row("SELECT kind FROM sessions WHERE id = ?1", [s1], |r| r.get(0)).unwrap();
    assert_eq!(s1_kind, "resumed", "S1 carried session_id of S0 on one record");
    let s0_kind: String = case.conn.query_row("SELECT kind FROM sessions WHERE id = ?1", [s0], |r| r.get(0)).unwrap();
    assert_eq!(s0_kind, "primary");
    let (status, matched): (String, Option<i64>) = case
        .conn
        .query_row("SELECT join_status, matched_session_id FROM identity_claims WHERE scheme = 'claude:origin_session_id' AND session_id = ?1", [s1], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((status.as_str(), matched), ("matched", Some(s0)));
    let (status, matched): (String, Option<i64>) = case
        .conn
        .query_row("SELECT join_status, matched_session_id FROM identity_claims WHERE scheme = 'claude:spawning_tool_use_id' AND session_id = ?1", [sub], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((status.as_str(), matched), ("matched", Some(s1)));
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies WHERE kind = 'unlinked_subagent'"), 0);
}

#[test]
fn finalizes_session_bounds_counts_and_models() {
    let (_tmp, mut case, _root) = attached_case();
    ingest(&mut case, None).unwrap();
    let s1 = session_id(&case, S1);
    let row: (Option<String>, String, Option<String>, String, i64, i64, String, Option<String>, Option<String>) = case
        .conn
        .query_row(
            "SELECT first_ts, first_ts_origin, last_ts, last_ts_origin, message_count, tool_call_count, models, title, project_path FROM sessions WHERE id = ?1",
            [s1],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
        )
        .unwrap();
    assert_eq!(row.0.as_deref(), Some("2026-09-30T10:00:00.000Z"));
    assert_eq!(row.1, "stored");
    assert_eq!(row.2.as_deref(), Some("2026-09-30T10:00:10.100Z"));
    assert_eq!(row.4, 11, "non-meta messages");
    assert_eq!(row.5, 4);
    assert_eq!(row.6, "[\"claude-fable-5-1\"]");
    assert_eq!(row.7.as_deref(), Some("Add notes file"));
    assert_eq!(row.8.as_deref(), Some("/home/alice/proj"));
    let sub = session_id(&case, "agent-0123456789abcdef");
    let title: Option<String> = case.conn.query_row("SELECT title FROM sessions WHERE id = ?1", [sub], |r| r.get(0)).unwrap();
    assert_eq!(title.as_deref(), Some("Find config files"));
}

#[test]
fn ingest_is_idempotent() {
    let (_tmp, mut case, _root) = attached_case();
    let first = ingest(&mut case, None).unwrap();
    let second = ingest(&mut case, None).unwrap();
    assert_eq!(second.files_parsed, 0);
    assert_eq!(second.files_skipped, first.files_parsed);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM sessions"), 4);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM messages"), 22);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM anomalies"), 4);
    assert_eq!(count(&case, "SELECT COUNT(*) FROM audit_log WHERE action = 'ingest'"), 2);
}

#[test]
fn unknown_root_filter_is_an_error_and_a_broken_file_is_marked_failed_without_stopping() {
    let (_tmp, mut case, root) = attached_case();
    assert!(ingest(&mut case, Some(root + 99)).is_err());
    // Simulate a file the adapter cannot open: point a source_files row at a path that does not exist.
    case.conn
        .execute(
            "INSERT INTO source_files (root_id, store_id, rel_path, size, sha256, parse_status, version) VALUES (?1, (SELECT id FROM stores WHERE kind = 'claude:projects'), 'projects/-home-alice-proj/ghost-0000-4000-8000-000000000009.jsonl', 0, 'x', 'unparsed', 1)",
            params![root],
        )
        .unwrap();
    let report = ingest(&mut case, Some(root)).unwrap();
    assert_eq!(report.files_failed, 1);
    assert_eq!(report.sessions, 5, "the ghost session row was created before the open failed");
    let (status, err): (String, Option<String>) = case
        .conn
        .query_row("SELECT parse_status, parse_error FROM source_files WHERE rel_path LIKE '%ghost%'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(status, "failed");
    assert!(err.unwrap().contains("io error"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-case --test ingest`
Expected: compile error, `ingest` module missing.

- [ ] **Step 3: Implement the database sink**

`crates/vem-case/src/sink.rs`:

```rust
//! `DbSink`: the `ParseSink` that writes canonical drafts into the case database, one file per transaction.

use crate::blobs;
use crate::error::CaseError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use vem_core::model::*;
use vem_core::sink::ParseSink;

#[derive(Debug, Default, Clone, Copy)]
pub struct SinkCounts {
    pub sessions: usize,
    pub messages: usize,
    pub tool_calls: usize,
    pub observations: usize,
    pub claims: usize,
    pub anomalies: usize,
}

pub struct DbSink<'a> {
    conn: &'a Connection,
    case_dir: &'a Path,
    root_id: i64,
    store_id: i64,
    source_file_id: i64,
    next_ordinal: HashMap<i64, i64>,
    pub counts: SinkCounts,
    /// First error hit inside a sink method; surfaced by `ingest` after `parse_file` returns.
    pub error: Option<CaseError>,
}

impl<'a> DbSink<'a> {
    pub fn new(conn: &'a Connection, case_dir: &'a Path, root_id: i64, store_id: i64, source_file_id: i64) -> Self {
        Self { conn, case_dir, root_id, store_id, source_file_id, next_ordinal: HashMap::new(), counts: SinkCounts::default(), error: None }
    }

    fn fail<T: Default>(&mut self, r: Result<T, CaseError>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => {
                if self.error.is_none() {
                    self.error = Some(e);
                }
                T::default()
            }
        }
    }

    fn insert_provenance(&self, p: &Provenance) -> Result<i64, CaseError> {
        self.conn.execute(
            "INSERT INTO provenance (source_file_id, byte_offset, byte_length, record_index, content_sha256, parser_name, parser_version, origin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![p.source_file.0, p.byte_offset as i64, p.byte_length as i64, p.record_index as i64, p.content_sha256, p.parser_name, p.parser_version, p.origin.as_str()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn block_id(&self, r: &BlockRef) -> Result<Option<i64>, CaseError> {
        Ok(self
            .conn
            .query_row("SELECT id FROM blocks WHERE message_id = ?1 AND ordinal = ?2", params![r.message.0, r.ordinal as i64], |row| row.get(0))
            .optional()?)
    }

    fn ordinal_for(&mut self, session: SessionHandle) -> Result<i64, CaseError> {
        if let Some(n) = self.next_ordinal.get_mut(&session.0) {
            let v = *n;
            *n += 1;
            return Ok(v);
        }
        let start: i64 = self.conn.query_row("SELECT COALESCE(MAX(ordinal), -1) + 1 FROM messages WHERE session_id = ?1", [session.0], |r| r.get(0))?;
        self.next_ordinal.insert(session.0, start + 1);
        Ok(start)
    }

    fn try_session(&mut self, d: &SessionDraft) -> Result<i64, CaseError> {
        let (first, first_o, last, last_o, fixed) = match (&d.first_ts, &d.last_ts) {
            (Some(f), Some(l)) => (f.value.clone(), f.origin.as_str(), l.value.clone(), l.origin.as_str(), 1),
            _ => (None, "absent", None, "absent", 0),
        };
        self.conn.execute(
            "INSERT INTO sessions (store_id, harness_session_id, kind, parent_harness_session_id, title, project_path, git_branch, harness_version, first_ts, first_ts_origin, last_ts, last_ts_origin, bounds_from_adapter, primary_source_file_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![self.store_id, d.harness_session_id, d.kind.as_str(), d.parent_harness_session_id, d.title, d.project_path, d.git_branch, d.harness_version, first, first_o, last, last_o, fixed, self.source_file_id],
        )?;
        self.counts.sessions += 1;
        Ok(self.conn.last_insert_rowid())
    }

    fn try_update(&mut self, session: SessionHandle, u: &SessionUpdate) -> Result<(), CaseError> {
        self.conn.execute(
            "UPDATE sessions SET title = COALESCE(?2, title), project_path = COALESCE(project_path, ?3), git_branch = COALESCE(git_branch, ?4), harness_version = COALESCE(harness_version, ?5) WHERE id = ?1",
            params![session.0, u.title, u.project_path, u.git_branch, u.harness_version],
        )?;
        if let Some(model) = &u.model {
            let raw: String = self.conn.query_row("SELECT models FROM sessions WHERE id = ?1", [session.0], |r| r.get(0))?;
            let mut models: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            if !models.contains(model) {
                models.push(model.clone());
                self.conn.execute("UPDATE sessions SET models = ?2 WHERE id = ?1", params![session.0, serde_json::to_string(&models)?])?;
            }
        }
        Ok(())
    }

    fn try_message(&mut self, session: SessionHandle, m: &MessageDraft) -> Result<i64, CaseError> {
        let prov_id = self.insert_provenance(&m.provenance)?;
        let ordinal = self.ordinal_for(session)?;
        self.conn.execute(
            "INSERT INTO messages (session_id, ordinal, role, harness_record_type, harness_uuid, parent_uuid, timestamp, ts_origin, model, provenance_id, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![session.0, ordinal, m.role.as_str(), m.harness_record_type, m.harness_uuid, m.parent_uuid, m.timestamp.value, m.timestamp.origin.as_str(), m.model, prov_id, Value::Object(m.attributes.clone()).to_string()],
        )?;
        let message_id = self.conn.last_insert_rowid();
        for (i, b) in m.blocks.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO blocks (message_id, ordinal, kind, text, payload, tool_use_id, provenance_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![message_id, i as i64, b.kind.as_str(), b.text, b.payload.to_string(), b.tool_use_id, prov_id],
            )?;
        }
        self.counts.messages += 1;
        Ok(message_id)
    }

    fn try_tool_call(&mut self, session: SessionHandle, t: &ToolCallDraft) -> Result<i64, CaseError> {
        let use_id = self.block_id(&t.tool_use)?.unwrap_or(0);
        let result_id = match &t.tool_result {
            Some(r) => self.block_id(r)?,
            None => None,
        };
        self.conn.execute(
            "INSERT INTO tool_calls (session_id, tool_use_block_id, tool_result_block_id, name, category, input, result_text, result_payload, is_error, started_ts, ended_ts, ts_origin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![session.0, use_id, result_id, t.name, t.category.as_str(), t.input.to_string(), t.result_text, t.result_payload.as_ref().map(|v| v.to_string()), t.is_error as i64, t.started.value, t.ended.value, t.started.origin.as_str()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute("UPDATE blocks SET tool_call_id = ?1 WHERE id = ?2 OR id = ?3", params![id, use_id, result_id])?;
        self.counts.tool_calls += 1;
        Ok(id)
    }

    fn try_observation(&mut self, session: SessionHandle, o: &ObservationDraft) -> Result<(), CaseError> {
        let (tc, blk, prov) = match &o.derived_from {
            Derivation::ToolCall(h) => (Some(h.0), None, None),
            Derivation::Block(r) => (None, self.block_id(r)?, None),
            Derivation::Record(p) => (None, None, Some(self.insert_provenance(p)?)),
        };
        self.conn.execute(
            "INSERT INTO observations (session_id, kind, derived_from_tool_call_id, derived_from_block_id, derived_from_provenance_id, path, command, before_blob, after_blob, timestamp, ts_origin, confidence, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![session.0, o.kind.as_str(), tc, blk, prov, o.path, o.command, o.before_blob, o.after_blob, o.timestamp.value, o.timestamp.origin.as_str(), o.confidence.as_str(), o.details.to_string()],
        )?;
        self.counts.observations += 1;
        Ok(())
    }

    fn try_claim(&mut self, session: SessionHandle, c: &IdentityClaimDraft) -> Result<(), CaseError> {
        self.conn.execute(
            "INSERT INTO identity_claims (session_id, scheme, claimed_id, source_file_id, join_status) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session.0, c.scheme, c.claimed_id, c.source_file.0, c.join_status.as_str()],
        )?;
        self.counts.claims += 1;
        Ok(())
    }

    fn try_anomaly(&mut self, a: &AnomalyDraft) -> Result<(), CaseError> {
        self.conn.execute(
            "INSERT INTO anomalies (root_id, store_id, source_file_id, session_id, kind, severity, byte_offset, message, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![self.root_id, self.store_id, a.source_file.map(|h| h.0).or(Some(self.source_file_id)), a.session.map(|h| h.0), a.kind.as_str(), a.severity.as_str(), a.byte_offset.map(|o| o as i64), a.message, a.details.to_string()],
        )?;
        self.counts.anomalies += 1;
        Ok(())
    }
}

impl<'a> ParseSink for DbSink<'a> {
    fn session(&mut self, draft: SessionDraft) -> SessionHandle {
        let r = self.try_session(&draft);
        let id = self.fail(r);
        SessionHandle(id)
    }
    fn update_session(&mut self, session: SessionHandle, update: SessionUpdate) {
        let r = self.try_update(session, &update);
        self.fail(r);
    }
    fn find_session(&self, harness_session_id: &str) -> Option<SessionHandle> {
        self.conn
            .query_row(
                "SELECT s.id FROM sessions s JOIN stores st ON st.id = s.store_id WHERE st.root_id = ?1 AND s.harness_session_id = ?2 ORDER BY s.id LIMIT 1",
                params![self.root_id, harness_session_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(SessionHandle)
    }
    fn message(&mut self, session: SessionHandle, draft: MessageDraft) -> MessageHandle {
        let r = self.try_message(session, &draft);
        let id = self.fail(r);
        MessageHandle(id)
    }
    fn tool_call(&mut self, session: SessionHandle, draft: ToolCallDraft) -> ToolCallHandle {
        let r = self.try_tool_call(session, &draft);
        let id = self.fail(r);
        ToolCallHandle(id)
    }
    fn observation(&mut self, session: SessionHandle, draft: ObservationDraft) {
        let r = self.try_observation(session, &draft);
        self.fail(r);
    }
    fn identity_claim(&mut self, session: SessionHandle, draft: IdentityClaimDraft) {
        let r = self.try_claim(session, &draft);
        self.fail(r);
    }
    fn anomaly(&mut self, draft: AnomalyDraft) {
        let r = self.try_anomaly(&draft);
        self.fail(r);
    }
    fn blob(&mut self, bytes: &[u8]) -> String {
        let r = blobs::put_bytes(self.conn, self.case_dir, bytes);
        self.fail(r)
    }
}
```

Every sink method binds the fallible result to a local before calling `fail`, because `self.fail(self.try_x(..))` would need two overlapping mutable borrows of `self`.

- [ ] **Step 4: Implement linking and finalization**

`crates/vem-case/src/link.rs`:

```rust
//! Post-ingest passes per evidence root: parent links, identity-claim resolution, session bounds.

use crate::error::CaseError;
use rusqlite::{params, Connection, OptionalExtension};

pub fn link_sessions(conn: &Connection, root_id: i64) -> Result<(), CaseError> {
    conn.execute(
        "UPDATE sessions SET parent_session_id = (
            SELECT p.id FROM sessions p JOIN stores ps ON ps.id = p.store_id
            WHERE ps.root_id = ?1 AND p.harness_session_id = sessions.parent_harness_session_id AND p.id != sessions.id
            ORDER BY p.id LIMIT 1)
         WHERE parent_harness_session_id IS NOT NULL AND parent_session_id IS NULL
           AND store_id IN (SELECT id FROM stores WHERE root_id = ?1)",
        [root_id],
    )?;

    let claims: Vec<(i64, i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.session_id, c.scheme, c.claimed_id FROM identity_claims c
             JOIN sessions s ON s.id = c.session_id JOIN stores st ON st.id = s.store_id
             WHERE st.root_id = ?1 AND c.join_status = 'unmatched' ORDER BY c.id",
        )?;
        let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (claim_id, session_id, scheme, claimed_id) in claims {
        let hits: Vec<i64> = if scheme == "claude:spawning_tool_use_id" {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT m.session_id FROM blocks b JOIN messages m ON m.id = b.message_id
                 JOIN sessions s ON s.id = m.session_id JOIN stores st ON st.id = s.store_id
                 WHERE st.root_id = ?1 AND b.kind = 'tool_use' AND b.tool_use_id = ?2",
            )?;
            let rows = stmt.query_map(params![root_id, claimed_id], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT s.id FROM sessions s JOIN stores st ON st.id = s.store_id
                 WHERE st.root_id = ?1 AND s.harness_session_id = ?2 AND s.id != ?3",
            )?;
            let rows = stmt.query_map(params![root_id, claimed_id, session_id], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        match hits.len() {
            0 => {}
            1 => {
                conn.execute("UPDATE identity_claims SET join_status = 'matched', matched_session_id = ?2 WHERE id = ?1", params![claim_id, hits[0]])?;
                if scheme == "claude:spawning_tool_use_id" {
                    conn.execute("UPDATE sessions SET parent_session_id = ?2 WHERE id = ?1 AND parent_session_id IS NULL", params![session_id, hits[0]])?;
                } else if scheme == "claude:origin_session_id" {
                    conn.execute("UPDATE sessions SET kind = 'resumed' WHERE id = ?1 AND kind = 'primary'", [session_id])?;
                }
            }
            _ => {
                conn.execute("UPDATE identity_claims SET join_status = 'ambiguous' WHERE id = ?1", [claim_id])?;
            }
        }
    }

    let unlinked: Vec<(i64, String, i64, Option<i64>)> = {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.harness_session_id, st.id, s.primary_source_file_id FROM sessions s JOIN stores st ON st.id = s.store_id
             WHERE st.root_id = ?1 AND s.kind = 'subagent' AND s.parent_session_id IS NULL
               AND NOT EXISTS (SELECT 1 FROM anomalies a WHERE a.session_id = s.id AND a.kind = 'unlinked_subagent')",
        )?;
        let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (sid, hid, store_id, file_id) in unlinked {
        conn.execute(
            "INSERT INTO anomalies (root_id, store_id, source_file_id, session_id, kind, severity, message, details) VALUES (?1, ?2, ?3, ?4, 'unlinked_subagent', 'warning', ?5, '{}')",
            params![root_id, store_id, file_id, sid, format!("subagent session {hid} has no parent transcript in this root")],
        )?;
    }
    Ok(())
}

pub fn finalize_sessions(conn: &Connection, root_id: i64) -> Result<(), CaseError> {
    let sessions: Vec<(i64, i64, Option<i64>)> = {
        let mut stmt = conn.prepare("SELECT s.id, s.bounds_from_adapter, s.primary_source_file_id FROM sessions s JOIN stores st ON st.id = s.store_id WHERE st.root_id = ?1")?;
        let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (sid, fixed, file_id) in sessions {
        if fixed == 0 {
            let first: Option<(String, String)> = conn
                .query_row("SELECT timestamp, ts_origin FROM messages WHERE session_id = ?1 AND timestamp IS NOT NULL ORDER BY timestamp ASC, ordinal ASC LIMIT 1", [sid], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let last: Option<(String, String)> = conn
                .query_row("SELECT timestamp, ts_origin FROM messages WHERE session_id = ?1 AND timestamp IS NOT NULL ORDER BY timestamp DESC, ordinal DESC LIMIT 1", [sid], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let (f, fo, l, lo) = match (first, last) {
                (Some((f, fo)), Some((l, lo))) => (Some(f), fo, Some(l), lo),
                _ => {
                    let mtime: Option<String> = match file_id {
                        Some(id) => conn.query_row("SELECT mtime FROM source_files WHERE id = ?1", [id], |r| r.get(0)).optional()?.flatten(),
                        None => None,
                    };
                    match mtime {
                        Some(m) => (Some(m.clone()), "file_mtime".to_string(), Some(m), "file_mtime".to_string()),
                        None => (None, "absent".to_string(), None, "absent".to_string()),
                    }
                }
            };
            conn.execute("UPDATE sessions SET first_ts = ?2, first_ts_origin = ?3, last_ts = ?4, last_ts_origin = ?5 WHERE id = ?1", params![sid, f, fo, l, lo])?;
        }
        let models: Vec<String> = {
            let mut stmt = conn.prepare("SELECT DISTINCT model FROM messages WHERE session_id = ?1 AND model IS NOT NULL ORDER BY model")?;
            let rows = stmt.query_map([sid], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        conn.execute(
            "UPDATE sessions SET message_count = (SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role != 'meta'),
                                 tool_call_count = (SELECT COUNT(*) FROM tool_calls WHERE session_id = ?1),
                                 models = ?2 WHERE id = ?1",
            params![sid, serde_json::to_string(&models)?],
        )?;
    }
    Ok(())
}
```

- [ ] **Step 5: Implement the ingest driver**

`crates/vem-case/src/ingest.rs`:

```rust
//! Ingest: parse every unparsed source file of every store, one transaction per file, then link and finalize.

use crate::case::now;
use crate::error::CaseError;
use crate::link::{finalize_sessions, link_sessions};
use crate::sink::DbSink;
use crate::Case;
use rusqlite::params;
use serde::Serialize;
use std::path::PathBuf;
use vem_core::adapter::{FileContext, StoreCandidate};
use vem_core::model::{Harness, SourceFileHandle};

#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestReport {
    pub files_parsed: usize,
    pub files_failed: usize,
    pub files_skipped: usize,
    pub sessions: usize,
    pub messages: usize,
    pub tool_calls: usize,
    pub observations: usize,
    pub anomalies: usize,
}

fn parse_ts_to_system_time(s: &str) -> Option<std::time::SystemTime> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(std::time::SystemTime::from)
}

pub fn ingest(case: &mut Case, root_filter: Option<i64>) -> Result<IngestReport, CaseError> {
    let roots: Vec<(i64, String, String)> = {
        let mut stmt = case.conn.prepare("SELECT id, path, harness FROM evidence_roots ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let roots: Vec<_> = roots.into_iter().filter(|(id, _, _)| root_filter.map(|f| f == *id).unwrap_or(true)).collect();
    if let Some(f) = root_filter {
        if roots.is_empty() {
            return Err(CaseError::NoSuchRoot(f));
        }
    }
    let mut report = IngestReport::default();
    let case_dir = case.dir.clone();

    for (root_id, root_path, harness) in roots {
        let harness = Harness::parse(&harness).ok_or_else(|| CaseError::NoAdapter(harness.clone()))?;
        let adapter = vem_adapters::adapter_for(harness).ok_or_else(|| CaseError::NoAdapter(harness.to_string()))?;
        let root = PathBuf::from(&root_path);
        let stores: Vec<(i64, String, Option<String>, String)> = {
            let mut stmt = case.conn.prepare("SELECT id, kind, generation, rel_path FROM stores WHERE root_id = ?1 ORDER BY id")?;
            let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (store_id, kind, generation, store_rel) in stores {
            let all_files: Vec<(i64, String, Option<String>, String)> = {
                let mut stmt = case.conn.prepare("SELECT id, rel_path, mtime, parse_status FROM source_files WHERE store_id = ?1 AND version = (SELECT MAX(version) FROM source_files g WHERE g.root_id = source_files.root_id AND g.rel_path = source_files.rel_path) ORDER BY rel_path")?;
                let rows = stmt.query_map([store_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            let candidate = StoreCandidate {
                kind: kind.clone(),
                generation,
                rel_path: PathBuf::from(&store_rel),
                files: all_files.iter().map(|(_, rel, _, _)| PathBuf::from(rel)).collect(),
            };
            let mut store_records = 0i64;
            for (file_id, rel, mtime, status) in &all_files {
                if status != "unparsed" {
                    report.files_skipped += 1;
                    continue;
                }
                let rel_path = PathBuf::from(rel);
                let ctx = FileContext {
                    root: &root,
                    store: &candidate,
                    rel_path: &rel_path,
                    abs_path: root.join(&rel_path),
                    handle: SourceFileHandle(*file_id),
                    mtime: mtime.as_deref().and_then(parse_ts_to_system_time),
                };
                let tx = case.conn.transaction()?;
                let outcome = {
                    let mut sink = DbSink::new(&tx, &case_dir, root_id, store_id, *file_id);
                    let parsed = adapter.parse_file(&ctx, &mut sink);
                    let counts = sink.counts;
                    let sink_error = sink.error.take();
                    (parsed, counts, sink_error)
                };
                let (parsed, counts, sink_error) = outcome;
                if let Some(e) = sink_error {
                    return Err(e);
                }
                report.sessions += counts.sessions;
                report.messages += counts.messages;
                report.tool_calls += counts.tool_calls;
                report.observations += counts.observations;
                report.anomalies += counts.anomalies;
                store_records += counts.messages as i64;
                match parsed {
                    Ok(()) => {
                        tx.execute(
                            "UPDATE source_files SET parse_status = 'parsed', parse_error = NULL, record_count = ?2, anomaly_count = ?3, ingested_at = ?4 WHERE id = ?1",
                            params![file_id, counts.messages as i64, counts.anomalies as i64, now()],
                        )?;
                        report.files_parsed += 1;
                    }
                    Err(e) => {
                        tx.execute(
                            "UPDATE source_files SET parse_status = 'failed', parse_error = ?2, record_count = ?3, anomaly_count = ?4, ingested_at = ?5 WHERE id = ?1",
                            params![file_id, e.to_string(), counts.messages as i64, counts.anomalies as i64, now()],
                        )?;
                        report.files_failed += 1;
                    }
                }
                tx.commit()?;
            }
            let has_records: i64 = case.conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE store_id = ?1",
                [store_id],
                |r| r.get(0),
            )?;
            let status = if has_records > 0 || store_records > 0 { "parsed" } else { "inventoried" };
            case.conn.execute("UPDATE stores SET status = ?2 WHERE id = ?1", params![store_id, status])?;
        }
        link_sessions(&case.conn, root_id)?;
        finalize_sessions(&case.conn, root_id)?;
    }
    case.audit("ingest", None, serde_json::to_value(&report)?)?;
    Ok(report)
}
```

Add to `crates/vem-case/src/lib.rs`:

```rust
pub mod ingest;
pub mod link;
pub mod sink;
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p vem-case --test ingest`
Expected: 5 passed. If `ingests_fixture_into_canonical_rows` disagrees on a count, print the rows (`SELECT kind, message FROM anomalies` / `SELECT role, harness_record_type FROM messages`) and reconcile against the fixture in Task 5 before changing any expectation: the fixture is the ground truth.

- [ ] **Step 7: Commit**

```bash
git add crates/vem-case
git commit -m "feat(case): ingest pipeline with database sink, claim resolution and session finalization

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: Verify integrity

**Files:**
- Create: `crates/vem-case/src/verify.rs`
- Create: `crates/vem-case/tests/verify.rs`
- Modify: `crates/vem-case/src/lib.rs`

**Interfaces:**
- Produces: `vem_case::verify::{verify(case: &mut Case) -> Result<VerifyReport, CaseError>, VerifyReport { files_checked, drifted: Vec<String>, missing: Vec<String>, roots_unavailable: Vec<String>, blobs_checked, blob_errors: Vec<String> }}`.

- [ ] **Step 1: Write the failing tests**

`crates/vem-case/tests/verify.rs`:

```rust
mod common;

use common::*;
use std::io::Write;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::verify::verify;
use vem_case::Case;

fn case_over_copy() -> (tempfile::TempDir, Case, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let ev = tmp.path().join("ev");
    copy_dir(&fixture_root(), &ev);
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &ev, AttachOptions { label: "l".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    (tmp, case, ev)
}

#[test]
fn clean_case_verifies_without_drift() {
    let (_tmp, mut case, _ev) = case_over_copy();
    let r = verify(&mut case).unwrap();
    assert!(r.files_checked > 0);
    assert!(r.drifted.is_empty() && r.missing.is_empty() && r.blob_errors.is_empty() && r.roots_unavailable.is_empty());
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM anomalies WHERE kind = 'hash_drift'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn detects_modified_and_missing_evidence_files() {
    let (_tmp, mut case, ev) = case_over_copy();
    std::fs::OpenOptions::new().append(true).open(ev.join("history.jsonl")).unwrap().write_all(b"\n").unwrap();
    std::fs::remove_file(ev.join("settings.json")).unwrap();
    let r = verify(&mut case).unwrap();
    assert_eq!(r.drifted, vec!["history.jsonl".to_string()]);
    assert_eq!(r.missing, vec!["settings.json".to_string()]);
    let n: i64 = case.conn.query_row("SELECT COUNT(*) FROM anomalies WHERE kind = 'hash_drift'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 2);
}

#[test]
fn detects_corrupted_retained_blob_and_detached_root() {
    let (_tmp, mut case, ev) = case_over_copy();
    let sha: String = case.conn.query_row("SELECT sha256 FROM source_files WHERE rel_path = 'history.jsonl'", [], |r| r.get(0)).unwrap();
    std::fs::write(case.blob_path(&sha), b"corrupted").unwrap();
    std::fs::remove_dir_all(&ev).unwrap();
    let r = verify(&mut case).unwrap();
    assert_eq!(r.roots_unavailable.len(), 1);
    assert_eq!(r.files_checked, 0);
    assert_eq!(r.blob_errors, vec![sha]);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-case --test verify`
Expected: compile error, `verify` module missing.

- [ ] **Step 3: Implement**

`crates/vem-case/src/verify.rs`:

```rust
//! Re-hash evidence files and retained blobs; record drift as `hash_drift` anomalies (spec §5).

use crate::error::CaseError;
use crate::{blobs, Case};
use rusqlite::params;
use serde::Serialize;
use std::path::PathBuf;
use vem_core::hash::sha256_file;

#[derive(Debug, Default, Clone, Serialize)]
pub struct VerifyReport {
    pub files_checked: usize,
    pub drifted: Vec<String>,
    pub missing: Vec<String>,
    pub roots_unavailable: Vec<String>,
    pub blobs_checked: usize,
    pub blob_errors: Vec<String>,
}

pub fn verify(case: &mut Case) -> Result<VerifyReport, CaseError> {
    let mut report = VerifyReport::default();
    let files: Vec<(i64, i64, String, String, String)> = {
        let mut stmt = case.conn.prepare(
            "SELECT f.id, r.id, r.path, f.rel_path, f.sha256 FROM source_files f JOIN evidence_roots r ON r.id = f.root_id
             WHERE f.version = (SELECT MAX(version) FROM source_files g WHERE g.root_id = f.root_id AND g.rel_path = f.rel_path)
             ORDER BY r.id, f.rel_path",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (file_id, root_id, root_path, rel, expected) in files {
        let root = PathBuf::from(&root_path);
        if !root.is_dir() {
            if !report.roots_unavailable.contains(&root_path) {
                report.roots_unavailable.push(root_path.clone());
            }
            continue;
        }
        let abs = root.join(&rel);
        report.files_checked += 1;
        match sha256_file(&abs) {
            Ok((actual, _)) if actual == expected => {}
            Ok((actual, _)) => {
                report.drifted.push(rel.clone());
                case.conn.execute(
                    "INSERT INTO anomalies (root_id, source_file_id, kind, severity, message, details) VALUES (?1, ?2, 'hash_drift', 'error', ?3, ?4)",
                    params![root_id, file_id, format!("{rel} no longer matches its manifest hash"), serde_json::json!({ "expected": expected, "actual": actual }).to_string()],
                )?;
            }
            Err(e) => {
                report.missing.push(rel.clone());
                case.conn.execute(
                    "INSERT INTO anomalies (root_id, source_file_id, kind, severity, message, details) VALUES (?1, ?2, 'hash_drift', 'warning', ?3, ?4)",
                    params![root_id, file_id, format!("{rel} is missing or unreadable in the evidence root"), serde_json::json!({ "expected": expected, "error": e.to_string() }).to_string()],
                )?;
            }
        }
    }
    let shas: Vec<String> = {
        let mut stmt = case.conn.prepare("SELECT sha256 FROM blobs ORDER BY sha256")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for sha in shas {
        report.blobs_checked += 1;
        let ok = matches!(sha256_file(&blobs::path(&case.dir, &sha)), Ok((actual, _)) if actual == sha);
        if !ok {
            report.blob_errors.push(sha.clone());
            case.conn.execute(
                "INSERT INTO anomalies (root_id, kind, severity, message, details) VALUES ((SELECT MIN(id) FROM evidence_roots), 'hash_drift', 'error', ?1, ?2)",
                params![format!("retained blob {sha} is missing or corrupted"), serde_json::json!({ "blob": sha }).to_string()],
            )?;
        }
    }
    case.audit("verify", None, serde_json::to_value(&report)?)?;
    Ok(report)
}
```

Add `pub mod verify;` to `crates/vem-case/src/lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p vem-case --test verify`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/vem-case
git commit -m "feat(case): verify evidence and retained blobs against the manifest

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 14: Query layer (what the CLI and the later web API read)

**Files:**
- Create: `crates/vem-case/src/query.rs`
- Create: `crates/vem-case/tests/query.rs`
- Modify: `crates/vem-case/src/lib.rs`

**Interfaces:**
- Produces (all in `vem_case::query`, all row structs `#[derive(Debug, Clone, Serialize)]`):
  - `RootRow { id, path, label, host, user, os, harness, attached_at }`, `roots(&Case) -> Result<Vec<RootRow>>`
  - `StoreRow { id, root_id, kind, generation, rel_path, discovery_method, status, file_count }`, `stores(&Case, root_id) -> Result<Vec<StoreRow>>`, `absent_stores(&Case, root_id) -> Result<Vec<String>>`
  - `SourceFileRow { id, root_id, store_id, rel_path, size, sha256, mtime, retained, parse_status, parse_error, record_count, anomaly_count }`, `source_files(&Case, root_id) -> Result<Vec<SourceFileRow>>`
  - `SessionRow { id, root_id, store_id, harness, harness_session_id, kind, parent_session_id, title, project_path, git_branch, harness_version, models: Vec<String>, first_ts, first_ts_origin, last_ts, last_ts_origin, message_count, tool_call_count, anomaly_count, child_count }`, `SessionFilter { root_id: Option<i64>, harness: Option<String>, kind: Option<String>, project_contains: Option<String> }`, `sessions(&Case, &SessionFilter) -> Result<Vec<SessionRow>>`, `session(&Case, id) -> Result<Option<SessionRow>>`, `children(&Case, id) -> Result<Vec<SessionRow>>`
  - `BlockRow { id, ordinal, kind, text, payload: Value, tool_use_id, tool_call_id }`, `MessageRow { id, session_id, ordinal, role, harness_record_type, harness_uuid, parent_uuid, timestamp, ts_origin, model, provenance_id, attributes: Value, blocks: Vec<BlockRow> }`, `messages(&Case, session_id, include_meta: bool) -> Result<Vec<MessageRow>>`
  - `ToolCallRow { id, session_id, name, category, input: Value, result_text, result_payload: Option<Value>, is_error, started_ts, ended_ts, ts_origin, tool_use_block_id, tool_result_block_id }`, `tool_calls(&Case, session_id) -> Result<Vec<ToolCallRow>>`
  - `ObservationRow { id, session_id, kind, path, command, before_blob, after_blob, timestamp, ts_origin, confidence, details: Value, derived_from_tool_call_id, derived_from_block_id, derived_from_provenance_id }`, `ObservationFilter { root_id, session_id, kind }`, `observations(&Case, &ObservationFilter) -> Result<Vec<ObservationRow>>`
  - `AnomalyRow { id, root_id, store_id, source_file_id, session_id, kind, severity, byte_offset, message, details: Value }`, `AnomalyFilter { root_id, kind, severity }`, `anomalies(&Case, &AnomalyFilter) -> Result<Vec<AnomalyRow>>`
  - `ClaimRow { id, session_id, scheme, claimed_id, source_file_id, join_status, matched_session_id }`, `claims(&Case, session_id) -> Result<Vec<ClaimRow>>`
  - `ProvenanceRow { id, source_file_id, rel_path, file_sha256, root_path, retained, byte_offset, byte_length, record_index, content_sha256, parser_name, parser_version, origin }`, `provenance(&Case, id) -> Result<Option<ProvenanceRow>>`, `raw_record(&Case, provenance_id) -> Result<Vec<u8>>` (reads the retained blob when present, otherwise the evidence file; verifies the bytes hash to `content_sha256` and returns `CaseError::Export("record bytes do not match provenance hash")` otherwise)
  - `SearchHit { block_id, message_id, session_id, snippet }`, `search(&Case, query: &str, limit: usize) -> Result<Vec<SearchHit>>`

- [ ] **Step 1: Write the failing tests**

`crates/vem-case/tests/query.rs`:

```rust
mod common;

use common::*;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::ingest::ingest;
use vem_case::query::*;
use vem_case::Case;

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &fixture_root(), AttachOptions { label: "alice".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    (tmp, case)
}

fn by_hid(case: &Case, hid: &str) -> SessionRow {
    sessions(case, &SessionFilter::default()).unwrap().into_iter().find(|s| s.harness_session_id == hid).unwrap()
}

#[test]
fn lists_roots_stores_and_files() {
    let (_t, case) = ingested();
    let r = roots(&case).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].label, "alice");
    let st = stores(&case, r[0].id).unwrap();
    assert_eq!(st[0].kind, "claude:projects");
    assert!(st[0].file_count >= 6);
    assert!(absent_stores(&case, r[0].id).unwrap().contains(&"claude:todos".to_string()));
    let files = source_files(&case, r[0].id).unwrap();
    assert!(files.iter().all(|f| f.parse_status == "parsed"));
}

#[test]
fn lists_sessions_with_counts_and_children() {
    let (_t, case) = ingested();
    let all = sessions(&case, &SessionFilter::default()).unwrap();
    assert_eq!(all.len(), 4);
    let order: Vec<&str> = all.iter().map(|s| s.harness_session_id.as_str()).collect();
    assert_eq!(order, vec!["22222222-0000-4000-8000-000000000002", "agent-0123456789abcdef", S1, S0], "newest first_ts first: orphan 11:00, subagent 10:00:07.2, S1 10:00:00, S0 the day before");
    let s1 = by_hid(&case, S1);
    assert_eq!(s1.kind, "resumed");
    assert_eq!(s1.message_count, 11);
    assert_eq!(s1.tool_call_count, 4);
    assert_eq!(s1.anomaly_count, 2);
    assert_eq!(s1.child_count, 1);
    assert_eq!(s1.models, vec!["claude-fable-5-1".to_string()]);
    let kids = children(&case, s1.id).unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].kind, "subagent");
    let only_sub = sessions(&case, &SessionFilter { kind: Some("subagent".into()), ..Default::default() }).unwrap();
    assert_eq!(only_sub.len(), 1);
    assert!(session(&case, 999_999).unwrap().is_none());
}

#[test]
fn reads_messages_blocks_tool_calls_observations_claims() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let conv = messages(&case, s1.id, false).unwrap();
    assert_eq!(conv.len(), 11);
    assert_eq!(conv[0].role, "user");
    assert_eq!(conv[0].blocks[0].text.as_deref(), Some("Create a notes file and list the repo"));
    assert_eq!(conv[1].blocks[1].kind, "tool_use");
    assert!(conv[1].blocks[1].tool_call_id.is_some());
    let with_meta = messages(&case, s1.id, true).unwrap();
    assert_eq!(with_meta.len(), 16);
    assert_eq!(with_meta[0].role, "meta");
    let tcs = tool_calls(&case, s1.id).unwrap();
    assert_eq!(tcs.len(), 4);
    assert_eq!(tcs[0].name, "Bash");
    assert_eq!(tcs[0].input["command"], "ls -la");
    let obs = observations(&case, &ObservationFilter { session_id: Some(s1.id), kind: Some("command_executed".into()), ..Default::default() }).unwrap();
    assert_eq!(obs.len(), 1);
    assert_eq!(obs[0].command.as_deref(), Some("ls -la"));
    let all_obs = observations(&case, &ObservationFilter::default()).unwrap();
    assert_eq!(all_obs.len(), 6);
    let cl = claims(&case, s1.id).unwrap();
    assert!(cl.iter().any(|c| c.scheme == "claude:origin_session_id" && c.join_status == "matched"));
    let an = anomalies(&case, &AnomalyFilter { severity: Some("warning".into()), ..Default::default() }).unwrap();
    assert!(an.iter().all(|a| a.severity == "warning"));
    assert!(an.iter().any(|a| a.kind == "missing_transcript"));
}

#[test]
fn provenance_and_raw_record_round_trip() {
    let (_t, case) = ingested();
    let s1 = by_hid(&case, S1);
    let first = &messages(&case, s1.id, true).unwrap()[0];
    let p = provenance(&case, first.provenance_id).unwrap().unwrap();
    assert_eq!(p.byte_offset, 0);
    assert!(p.rel_path.ends_with("000000000001.jsonl"));
    assert!(p.retained);
    let bytes = raw_record(&case, first.provenance_id).unwrap();
    assert!(bytes.starts_with(b"{\"type\":\"ai-title\""));
    assert_eq!(vem_core::hash::sha256_hex(&bytes), p.content_sha256);
}

#[test]
fn full_text_search_finds_blocks() {
    let (_t, case) = ingested();
    let hits = search(&case, "notes", 10).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.snippet.to_lowercase().contains("notes")));
    assert!(search(&case, "zzzz-nothing-here", 10).unwrap().is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-case --test query`
Expected: compile error, `query` module missing.

- [ ] **Step 3: Implement**

`crates/vem-case/src/query.rs`:

```rust
//! Read side of the case database. Every struct here is what the CLI prints and the web API will serialize.

use crate::error::CaseError;
use crate::{blobs, Case};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use vem_core::hash::sha256_hex;

fn json(s: String) -> Value {
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

#[derive(Debug, Clone, Serialize)]
pub struct RootRow { pub id: i64, pub path: String, pub label: String, pub host: Option<String>, pub user: Option<String>, pub os: Option<String>, pub harness: String, pub attached_at: String }

pub fn roots(case: &Case) -> Result<Vec<RootRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, path, label, host, user, os, harness, attached_at FROM evidence_roots ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok(RootRow { id: r.get(0)?, path: r.get(1)?, label: r.get(2)?, host: r.get(3)?, user: r.get(4)?, os: r.get(5)?, harness: r.get(6)?, attached_at: r.get(7)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreRow { pub id: i64, pub root_id: i64, pub kind: String, pub generation: Option<String>, pub rel_path: String, pub discovery_method: String, pub status: String, pub file_count: i64 }

pub fn stores(case: &Case, root_id: i64) -> Result<Vec<StoreRow>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT s.id, s.root_id, s.kind, s.generation, s.rel_path, s.discovery_method, s.status, (SELECT COUNT(*) FROM source_files f WHERE f.store_id = s.id) FROM stores s WHERE s.root_id = ?1 ORDER BY s.id",
    )?;
    let rows = stmt.query_map([root_id], |r| Ok(StoreRow { id: r.get(0)?, root_id: r.get(1)?, kind: r.get(2)?, generation: r.get(3)?, rel_path: r.get(4)?, discovery_method: r.get(5)?, status: r.get(6)?, file_count: r.get(7)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn absent_stores(case: &Case, root_id: i64) -> Result<Vec<String>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT kind FROM absent_stores WHERE root_id = ?1 ORDER BY kind")?;
    let rows = stmt.query_map([root_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceFileRow { pub id: i64, pub root_id: i64, pub store_id: Option<i64>, pub rel_path: String, pub size: i64, pub sha256: String, pub mtime: Option<String>, pub retained: bool, pub parse_status: String, pub parse_error: Option<String>, pub record_count: i64, pub anomaly_count: i64 }

pub fn source_files(case: &Case, root_id: i64) -> Result<Vec<SourceFileRow>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT id, root_id, store_id, rel_path, size, sha256, mtime, retained, parse_status, parse_error, record_count, anomaly_count FROM source_files WHERE root_id = ?1 ORDER BY rel_path, version",
    )?;
    let rows = stmt.query_map([root_id], |r| Ok(SourceFileRow { id: r.get(0)?, root_id: r.get(1)?, store_id: r.get(2)?, rel_path: r.get(3)?, size: r.get(4)?, sha256: r.get(5)?, mtime: r.get(6)?, retained: r.get::<_, i64>(7)? == 1, parse_status: r.get(8)?, parse_error: r.get(9)?, record_count: r.get(10)?, anomaly_count: r.get(11)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub id: i64, pub root_id: i64, pub store_id: i64, pub harness: String, pub harness_session_id: String, pub kind: String,
    pub parent_session_id: Option<i64>, pub title: Option<String>, pub project_path: Option<String>, pub git_branch: Option<String>,
    pub harness_version: Option<String>, pub models: Vec<String>, pub first_ts: Option<String>, pub first_ts_origin: String,
    pub last_ts: Option<String>, pub last_ts_origin: String, pub message_count: i64, pub tool_call_count: i64, pub anomaly_count: i64, pub child_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct SessionFilter { pub root_id: Option<i64>, pub harness: Option<String>, pub kind: Option<String>, pub project_contains: Option<String> }

const SESSION_SELECT: &str = "SELECT s.id, st.root_id, s.store_id, st.harness, s.harness_session_id, s.kind, s.parent_session_id, s.title, s.project_path, s.git_branch, s.harness_version, s.models, s.first_ts, s.first_ts_origin, s.last_ts, s.last_ts_origin, s.message_count, s.tool_call_count,
    (SELECT COUNT(*) FROM anomalies a WHERE a.session_id = s.id), (SELECT COUNT(*) FROM sessions c WHERE c.parent_session_id = s.id)
    FROM sessions s JOIN stores st ON st.id = s.store_id";

fn session_row(r: &Row<'_>) -> rusqlite::Result<SessionRow> {
    let models: String = r.get(11)?;
    Ok(SessionRow {
        id: r.get(0)?, root_id: r.get(1)?, store_id: r.get(2)?, harness: r.get(3)?, harness_session_id: r.get(4)?, kind: r.get(5)?,
        parent_session_id: r.get(6)?, title: r.get(7)?, project_path: r.get(8)?, git_branch: r.get(9)?, harness_version: r.get(10)?,
        models: serde_json::from_str(&models).unwrap_or_default(), first_ts: r.get(12)?, first_ts_origin: r.get(13)?, last_ts: r.get(14)?,
        last_ts_origin: r.get(15)?, message_count: r.get(16)?, tool_call_count: r.get(17)?, anomaly_count: r.get(18)?, child_count: r.get(19)?,
    })
}

pub fn sessions(case: &Case, f: &SessionFilter) -> Result<Vec<SessionRow>, CaseError> {
    let mut sql = format!("{SESSION_SELECT} WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id { sql.push_str(" AND st.root_id = ?"); args.push(Box::new(r)); }
    if let Some(h) = &f.harness { sql.push_str(" AND st.harness = ?"); args.push(Box::new(h.clone())); }
    if let Some(k) = &f.kind { sql.push_str(" AND s.kind = ?"); args.push(Box::new(k.clone())); }
    if let Some(p) = &f.project_contains { sql.push_str(" AND s.project_path LIKE ?"); args.push(Box::new(format!("%{p}%"))); }
    sql.push_str(" ORDER BY s.first_ts DESC, s.id DESC");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), session_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn session(case: &Case, id: i64) -> Result<Option<SessionRow>, CaseError> {
    Ok(case.conn.query_row(&format!("{SESSION_SELECT} WHERE s.id = ?1"), [id], session_row).optional()?)
}

pub fn children(case: &Case, id: i64) -> Result<Vec<SessionRow>, CaseError> {
    let mut stmt = case.conn.prepare(&format!("{SESSION_SELECT} WHERE s.parent_session_id = ?1 ORDER BY s.first_ts, s.id"))?;
    let rows = stmt.query_map([id], session_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockRow { pub id: i64, pub ordinal: i64, pub kind: String, pub text: Option<String>, pub payload: Value, pub tool_use_id: Option<String>, pub tool_call_id: Option<i64> }

#[derive(Debug, Clone, Serialize)]
pub struct MessageRow {
    pub id: i64, pub session_id: i64, pub ordinal: i64, pub role: String, pub harness_record_type: String, pub harness_uuid: Option<String>,
    pub parent_uuid: Option<String>, pub timestamp: Option<String>, pub ts_origin: String, pub model: Option<String>, pub provenance_id: i64,
    pub attributes: Value, pub blocks: Vec<BlockRow>,
}

pub fn messages(case: &Case, session_id: i64, include_meta: bool) -> Result<Vec<MessageRow>, CaseError> {
    let sql = format!(
        "SELECT id, session_id, ordinal, role, harness_record_type, harness_uuid, parent_uuid, timestamp, ts_origin, model, provenance_id, attributes FROM messages WHERE session_id = ?1 {} ORDER BY ordinal",
        if include_meta { "" } else { "AND role != 'meta'" }
    );
    let mut stmt = case.conn.prepare(&sql)?;
    let mut out: Vec<MessageRow> = stmt
        .query_map([session_id], |r| {
            let attrs: String = r.get(11)?;
            Ok(MessageRow { id: r.get(0)?, session_id: r.get(1)?, ordinal: r.get(2)?, role: r.get(3)?, harness_record_type: r.get(4)?, harness_uuid: r.get(5)?, parent_uuid: r.get(6)?, timestamp: r.get(7)?, ts_origin: r.get(8)?, model: r.get(9)?, provenance_id: r.get(10)?, attributes: json(attrs), blocks: Vec::new() })
        })?
        .collect::<Result<_, _>>()?;
    let mut bstmt = case.conn.prepare("SELECT id, ordinal, kind, text, payload, tool_use_id, tool_call_id FROM blocks WHERE message_id = ?1 ORDER BY ordinal")?;
    for m in &mut out {
        let blocks = bstmt.query_map([m.id], |r| {
            let payload: String = r.get(4)?;
            Ok(BlockRow { id: r.get(0)?, ordinal: r.get(1)?, kind: r.get(2)?, text: r.get(3)?, payload: json(payload), tool_use_id: r.get(5)?, tool_call_id: r.get(6)? })
        })?;
        m.blocks = blocks.collect::<Result<_, _>>()?;
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRow {
    pub id: i64, pub session_id: i64, pub name: String, pub category: String, pub input: Value, pub result_text: Option<String>,
    pub result_payload: Option<Value>, pub is_error: bool, pub started_ts: Option<String>, pub ended_ts: Option<String>, pub ts_origin: String,
    pub tool_use_block_id: i64, pub tool_result_block_id: Option<i64>,
}

pub fn tool_calls(case: &Case, session_id: i64) -> Result<Vec<ToolCallRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, session_id, name, category, input, result_text, result_payload, is_error, started_ts, ended_ts, ts_origin, tool_use_block_id, tool_result_block_id FROM tool_calls WHERE session_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([session_id], |r| {
        let input: String = r.get(4)?;
        let payload: Option<String> = r.get(6)?;
        Ok(ToolCallRow { id: r.get(0)?, session_id: r.get(1)?, name: r.get(2)?, category: r.get(3)?, input: json(input), result_text: r.get(5)?, result_payload: payload.map(json), is_error: r.get::<_, i64>(7)? == 1, started_ts: r.get(8)?, ended_ts: r.get(9)?, ts_origin: r.get(10)?, tool_use_block_id: r.get(11)?, tool_result_block_id: r.get(12)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationRow {
    pub id: i64, pub session_id: i64, pub kind: String, pub path: Option<String>, pub command: Option<String>, pub before_blob: Option<String>,
    pub after_blob: Option<String>, pub timestamp: Option<String>, pub ts_origin: String, pub confidence: String, pub details: Value,
    pub derived_from_tool_call_id: Option<i64>, pub derived_from_block_id: Option<i64>, pub derived_from_provenance_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ObservationFilter { pub root_id: Option<i64>, pub session_id: Option<i64>, pub kind: Option<String> }

pub fn observations(case: &Case, f: &ObservationFilter) -> Result<Vec<ObservationRow>, CaseError> {
    let mut sql = String::from("SELECT o.id, o.session_id, o.kind, o.path, o.command, o.before_blob, o.after_blob, o.timestamp, o.ts_origin, o.confidence, o.details, o.derived_from_tool_call_id, o.derived_from_block_id, o.derived_from_provenance_id FROM observations o JOIN sessions s ON s.id = o.session_id JOIN stores st ON st.id = s.store_id WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id { sql.push_str(" AND st.root_id = ?"); args.push(Box::new(r)); }
    if let Some(s) = f.session_id { sql.push_str(" AND o.session_id = ?"); args.push(Box::new(s)); }
    if let Some(k) = &f.kind { sql.push_str(" AND o.kind = ?"); args.push(Box::new(k.clone())); }
    sql.push_str(" ORDER BY o.timestamp, o.id");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
        let details: String = r.get(10)?;
        Ok(ObservationRow { id: r.get(0)?, session_id: r.get(1)?, kind: r.get(2)?, path: r.get(3)?, command: r.get(4)?, before_blob: r.get(5)?, after_blob: r.get(6)?, timestamp: r.get(7)?, ts_origin: r.get(8)?, confidence: r.get(9)?, details: json(details), derived_from_tool_call_id: r.get(11)?, derived_from_block_id: r.get(12)?, derived_from_provenance_id: r.get(13)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct AnomalyRow { pub id: i64, pub root_id: i64, pub store_id: Option<i64>, pub source_file_id: Option<i64>, pub session_id: Option<i64>, pub kind: String, pub severity: String, pub byte_offset: Option<i64>, pub message: String, pub details: Value }

#[derive(Debug, Clone, Default)]
pub struct AnomalyFilter { pub root_id: Option<i64>, pub kind: Option<String>, pub severity: Option<String> }

pub fn anomalies(case: &Case, f: &AnomalyFilter) -> Result<Vec<AnomalyRow>, CaseError> {
    let mut sql = String::from("SELECT id, root_id, store_id, source_file_id, session_id, kind, severity, byte_offset, message, details FROM anomalies WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id { sql.push_str(" AND root_id = ?"); args.push(Box::new(r)); }
    if let Some(k) = &f.kind { sql.push_str(" AND kind = ?"); args.push(Box::new(k.clone())); }
    if let Some(s) = &f.severity { sql.push_str(" AND severity = ?"); args.push(Box::new(s.clone())); }
    sql.push_str(" ORDER BY id");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
        let details: String = r.get(9)?;
        Ok(AnomalyRow { id: r.get(0)?, root_id: r.get(1)?, store_id: r.get(2)?, source_file_id: r.get(3)?, session_id: r.get(4)?, kind: r.get(5)?, severity: r.get(6)?, byte_offset: r.get(7)?, message: r.get(8)?, details: json(details) })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimRow { pub id: i64, pub session_id: i64, pub scheme: String, pub claimed_id: String, pub source_file_id: i64, pub join_status: String, pub matched_session_id: Option<i64> }

pub fn claims(case: &Case, session_id: i64) -> Result<Vec<ClaimRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, session_id, scheme, claimed_id, source_file_id, join_status, matched_session_id FROM identity_claims WHERE session_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([session_id], |r| Ok(ClaimRow { id: r.get(0)?, session_id: r.get(1)?, scheme: r.get(2)?, claimed_id: r.get(3)?, source_file_id: r.get(4)?, join_status: r.get(5)?, matched_session_id: r.get(6)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceRow {
    pub id: i64, pub source_file_id: i64, pub rel_path: String, pub file_sha256: String, pub root_path: String, pub retained: bool,
    pub byte_offset: i64, pub byte_length: i64, pub record_index: i64, pub content_sha256: String, pub parser_name: String, pub parser_version: String, pub origin: String,
}

pub fn provenance(case: &Case, id: i64) -> Result<Option<ProvenanceRow>, CaseError> {
    Ok(case
        .conn
        .query_row(
            "SELECT p.id, p.source_file_id, f.rel_path, f.sha256, r.path, f.retained, p.byte_offset, p.byte_length, p.record_index, p.content_sha256, p.parser_name, p.parser_version, p.origin
             FROM provenance p JOIN source_files f ON f.id = p.source_file_id JOIN evidence_roots r ON r.id = f.root_id WHERE p.id = ?1",
            [id],
            |r| Ok(ProvenanceRow { id: r.get(0)?, source_file_id: r.get(1)?, rel_path: r.get(2)?, file_sha256: r.get(3)?, root_path: r.get(4)?, retained: r.get::<_, i64>(5)? == 1, byte_offset: r.get(6)?, byte_length: r.get(7)?, record_index: r.get(8)?, content_sha256: r.get(9)?, parser_name: r.get(10)?, parser_version: r.get(11)?, origin: r.get(12)? }),
        )
        .optional()?)
}

/// The exact bytes a provenance row points at, from the retained copy when there is one.
pub fn raw_record(case: &Case, provenance_id: i64) -> Result<Vec<u8>, CaseError> {
    let p = provenance(case, provenance_id)?.ok_or_else(|| CaseError::Export(format!("no provenance row {provenance_id}")))?;
    let path = if p.retained { blobs::path(&case.dir, &p.file_sha256) } else { std::path::Path::new(&p.root_path).join(&p.rel_path) };
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(p.byte_offset as u64))?;
    let mut buf = vec![0u8; p.byte_length as usize];
    f.read_exact(&mut buf)?;
    if sha256_hex(&buf) != p.content_sha256 {
        return Err(CaseError::Export("record bytes do not match provenance hash".to_string()));
    }
    Ok(buf)
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit { pub block_id: i64, pub message_id: i64, pub session_id: i64, pub snippet: String }

pub fn search(case: &Case, query: &str, limit: usize) -> Result<Vec<SearchHit>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT b.id, b.message_id, m.session_id, snippet(blocks_fts, 0, '[', ']', '…', 12)
         FROM blocks_fts JOIN blocks b ON b.id = blocks_fts.rowid JOIN messages m ON m.id = b.message_id
         WHERE blocks_fts MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;
    let quoted = format!("\"{}\"", query.replace('"', "\"\""));
    let rows = stmt.query_map(params![quoted, limit as i64], |r| Ok(SearchHit { block_id: r.get(0)?, message_id: r.get(1)?, session_id: r.get(2)?, snippet: r.get(3)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}
```

Add `pub mod query;` to `crates/vem-case/src/lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p vem-case --test query`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/vem-case
git commit -m "feat(case): query layer for roots, sessions, messages, tool calls, observations, provenance and search

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 15: Timesketch-compatible JSONL and CSV export

**Files:**
- Create: `crates/vem-case/src/export/mod.rs`
- Create: `crates/vem-case/src/export/events.rs`
- Create: `crates/vem-case/src/export/timesketch.rs`
- Create: `crates/vem-case/tests/export_timesketch.rs`
- Modify: `crates/vem-case/src/lib.rs`

**Interfaces:**
- Produces: `vem_case::export::{Scope { Case, Root(i64), Session(i64) }, Event { datetime: Option<String>, timestamp_desc: String, message: String, source: String, source_long: String, display_name: String, tags: Vec<String>, attributes: BTreeMap<String, String>, provenance: EventProvenance { source_file: String, file_sha256: String, byte_offset: u64, content_sha256: String, file_size: u64, file_mtime: Option<String> } }, events(case, &Scope) -> Result<Vec<Event>>, timestamp_desc(what: &str, origin: &str) -> String}`; `vem_case::export::timesketch::{write_jsonl(events: &[Event], w: impl Write) -> Result<()>, write_csv(events: &[Event], w: impl Write) -> Result<()>}`.
- Attribute keys (strings): `harness`, `root_label`, `session_db_id`, `session_id`, `session_kind`, `project_path`, `record_kind` (`message` | `tool_call` | `observation`), `role`, `record_type`, `message_uuid`, `model`, `tool_name`, `tool_category`, `observation_kind`, `path`, `command`, `confidence`, `ts_origin`, `evidence_file`, `evidence_file_sha256`, `evidence_byte_offset`, `evidence_record_sha256`. Absent values are omitted from the map.

- [ ] **Step 1: Write the failing tests**

`crates/vem-case/tests/export_timesketch.rs`:

```rust
mod common;

use common::*;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::timesketch::{write_csv, write_jsonl};
use vem_case::export::{events, Scope};
use vem_case::ingest::ingest;
use vem_case::query::{sessions, SessionFilter};
use vem_case::Case;

fn ingested() -> (tempfile::TempDir, Case) {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &fixture_root(), AttachOptions { label: "alice".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    (tmp, case)
}

#[test]
fn one_event_per_message_tool_call_and_observation() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    assert_eq!(ev.len(), 22 + 4 + 6);
    let first = ev.iter().find(|e| e.attributes.get("record_type").map(String::as_str) == Some("ai-title")).unwrap();
    assert_eq!(first.datetime, None);
    assert_eq!(first.timestamp_desc, "No Timestamp");
    assert_eq!(first.source, "AI:CLAUDE_CODE");
    assert_eq!(first.source_long, "claude_code:message:meta");
    assert!(first.display_name.starts_with("alice:projects/"));
    assert!(first.tags.contains(&"meta".to_string()) && first.tags.contains(&"absent".to_string()));
    assert_eq!(first.provenance.byte_offset, 0);
    let cmd = ev.iter().find(|e| e.attributes.get("observation_kind").map(String::as_str) == Some("command_executed")).unwrap();
    assert_eq!(cmd.datetime.as_deref(), Some("2026-09-30T10:00:02.000Z"));
    assert_eq!(cmd.timestamp_desc, "Observation Time (stored)");
    assert_eq!(cmd.attributes["command"], "ls -la");
    assert!(cmd.message.contains("ls -la"));
    let tc = ev.iter().find(|e| e.attributes.get("tool_name").map(String::as_str) == Some("Bash")).unwrap();
    assert_eq!(tc.timestamp_desc, "Tool Call Started (stored)");
    assert_eq!(tc.source_long, "claude_code:tool_call:shell");
    let user = ev.iter().find(|e| e.attributes.get("role").map(String::as_str) == Some("user")).unwrap();
    assert_eq!(user.message, "[claude-code] user: Create a notes file and list the repo");
    assert_eq!(user.timestamp_desc, "Message Timestamp (stored)");
    assert!(ev.windows(2).all(|w| w[0].datetime.is_none() || w[1].datetime.is_none() || w[0].datetime <= w[1].datetime), "sorted by time");
}

#[test]
fn scopes_restrict_events() {
    let (_t, case) = ingested();
    let s0 = sessions(&case, &SessionFilter::default()).unwrap().into_iter().find(|s| s.harness_session_id == S0).unwrap();
    let ev = events(&case, &Scope::Session(s0.id)).unwrap();
    assert_eq!(ev.len(), 2);
    let ev = events(&case, &Scope::Root(s0.root_id)).unwrap();
    assert_eq!(ev.len(), 32);
}

#[test]
fn jsonl_lines_carry_timesketch_fields_and_attributes_flattened() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    let mut buf = Vec::new();
    write_jsonl(&ev, &mut buf).unwrap();
    let lines: Vec<serde_json::Value> = String::from_utf8(buf).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), ev.len());
    let with_time = lines.iter().find(|l| !l["datetime"].is_null()).unwrap();
    for key in ["datetime", "timestamp_desc", "message", "source", "source_long", "display_name", "tags", "evidence_file_sha256", "evidence_byte_offset", "evidence_record_sha256"] {
        assert!(with_time.get(key).is_some(), "missing {key}");
    }
    assert!(with_time["tags"].is_array());
    assert!(with_time["evidence_byte_offset"].is_string(), "attributes are strings");
}

#[test]
fn csv_has_union_header_and_one_row_per_event() {
    let (_t, case) = ingested();
    let ev = events(&case, &Scope::Case).unwrap();
    let mut buf = Vec::new();
    write_csv(&ev, &mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();
    let mut rdr = csv::Reader::from_reader(text.as_bytes());
    let headers: Vec<String> = rdr.headers().unwrap().iter().map(String::from).collect();
    assert_eq!(&headers[..7], &["datetime", "timestamp_desc", "message", "source", "source_long", "display_name", "tag"]);
    assert!(headers.contains(&"command".to_string()));
    let rows: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
    assert_eq!(rows.len(), ev.len());
    assert!(rows.iter().all(|r| r.len() == headers.len()));
    let tagged = rows.iter().find(|r| r[6].contains('|')).expect("pipe-joined tags");
    assert!(tagged[6].contains("claude-code"));
}
```

Add `csv.workspace = true` to `[dev-dependencies]` of `crates/vem-case/Cargo.toml` (it is already a normal dependency, so the test can use it without that; skip).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-case --test export_timesketch`
Expected: compile error, `export` module missing.

- [ ] **Step 3: Implement events**

`crates/vem-case/src/export/mod.rs`:

```rust
//! Exports (spec §9): a common event list, written as Timesketch JSONL/CSV or Vestigo Parquet.

pub mod events;
pub mod timesketch;

pub use events::{events, timestamp_desc, Event, EventProvenance, Scope};
```

`crates/vem-case/src/export/events.rs`:

```rust
//! One `Event` per message, tool call and observation, with provenance for Vestigo.

use crate::error::CaseError;
use crate::Case;
use rusqlite::params_from_iter;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Case,
    Root(i64),
    Session(i64),
}

#[derive(Debug, Clone, Serialize)]
pub struct EventProvenance {
    pub source_file: String,
    pub file_sha256: String,
    pub byte_offset: u64,
    pub content_sha256: String,
    pub file_size: u64,
    pub file_mtime: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub datetime: Option<String>,
    pub timestamp_desc: String,
    pub message: String,
    pub source: String,
    pub source_long: String,
    pub display_name: String,
    pub tags: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub provenance: EventProvenance,
}

pub fn timestamp_desc(what: &str, origin: &str) -> String {
    match origin {
        "absent" => "No Timestamp".to_string(),
        "stored" => format!("{what} (stored)"),
        "stored_local_clock" => format!("{what} (stored, local clock)"),
        "file_mtime" => "Session File Modified (inferred)".to_string(),
        "neighbor_interpolated" => format!("{what} (interpolated, inferred)"),
        other => format!("{what} ({other})"),
    }
}

fn source_for(harness: &str) -> (String, String) {
    let upper = harness.replace('-', "_").to_uppercase();
    (format!("AI:{upper}"), harness.replace('-', "_"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

fn scope_clause(scope: &Scope, args: &mut Vec<Box<dyn rusqlite::ToSql>>) -> String {
    match scope {
        Scope::Case => String::new(),
        Scope::Root(r) => { args.push(Box::new(*r)); " AND st.root_id = ?".to_string() }
        Scope::Session(s) => { args.push(Box::new(*s)); " AND s.id = ?".to_string() }
    }
}

fn put(attrs: &mut BTreeMap<String, String>, key: &str, v: Option<String>) {
    if let Some(v) = v {
        if !v.is_empty() {
            attrs.insert(key.to_string(), v);
        }
    }
}

struct Common {
    harness: String, root_label: String, session_db_id: i64, session_hid: String, session_kind: String, project_path: Option<String>,
    rel_path: String, file_sha: String, file_size: i64, file_mtime: Option<String>, byte_offset: i64, content_sha: String,
}

impl Common {
    fn base(&self, record_kind: &str, ts_origin: &str) -> (BTreeMap<String, String>, EventProvenance, String) {
        let mut a = BTreeMap::new();
        put(&mut a, "harness", Some(self.harness.clone()));
        put(&mut a, "root_label", Some(self.root_label.clone()));
        put(&mut a, "session_db_id", Some(self.session_db_id.to_string()));
        put(&mut a, "session_id", Some(self.session_hid.clone()));
        put(&mut a, "session_kind", Some(self.session_kind.clone()));
        put(&mut a, "project_path", self.project_path.clone());
        put(&mut a, "record_kind", Some(record_kind.to_string()));
        put(&mut a, "ts_origin", Some(ts_origin.to_string()));
        put(&mut a, "evidence_file", Some(self.rel_path.clone()));
        put(&mut a, "evidence_file_sha256", Some(self.file_sha.clone()));
        put(&mut a, "evidence_byte_offset", Some(self.byte_offset.to_string()));
        put(&mut a, "evidence_record_sha256", Some(self.content_sha.clone()));
        let prov = EventProvenance { source_file: self.rel_path.clone(), file_sha256: self.file_sha.clone(), byte_offset: self.byte_offset as u64, content_sha256: self.content_sha.clone(), file_size: self.file_size as u64, file_mtime: self.file_mtime.clone() };
        (a, prov, format!("{}:{}", self.root_label, self.rel_path))
    }
}

const COMMON_COLS: &str = "st.harness, r.label, s.id, s.harness_session_id, s.kind, s.project_path, f.rel_path, f.sha256, f.size, f.mtime, p.byte_offset, p.content_sha256";

fn common_from(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Common> {
    Ok(Common {
        harness: r.get(offset)?, root_label: r.get(offset + 1)?, session_db_id: r.get(offset + 2)?, session_hid: r.get(offset + 3)?, session_kind: r.get(offset + 4)?,
        project_path: r.get(offset + 5)?, rel_path: r.get(offset + 6)?, file_sha: r.get(offset + 7)?, file_size: r.get(offset + 8)?, file_mtime: r.get(offset + 9)?,
        byte_offset: r.get(offset + 10)?, content_sha: r.get(offset + 11)?,
    })
}

pub fn events(case: &Case, scope: &Scope) -> Result<Vec<Event>, CaseError> {
    let mut out = Vec::new();

    // Messages: text is the concatenation of the message's text blocks.
    {
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let clause = scope_clause(scope, &mut args);
        let sql = format!(
            "SELECT m.id, m.role, m.harness_record_type, m.harness_uuid, m.timestamp, m.ts_origin, m.model,
                    (SELECT GROUP_CONCAT(b.text, char(10)) FROM blocks b WHERE b.message_id = m.id AND b.kind = 'text' AND b.text IS NOT NULL),
                    {COMMON_COLS}
             FROM messages m JOIN sessions s ON s.id = m.session_id JOIN stores st ON st.id = s.store_id JOIN evidence_roots r ON r.id = st.root_id
                  JOIN provenance p ON p.id = m.provenance_id JOIN source_files f ON f.id = p.source_file_id
             WHERE 1 = 1 {clause} ORDER BY m.id"
        );
        let mut stmt = case.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
            let role: String = r.get(1)?;
            let record_type: String = r.get(2)?;
            let uuid: Option<String> = r.get(3)?;
            let ts: Option<String> = r.get(4)?;
            let origin: String = r.get(5)?;
            let model: Option<String> = r.get(6)?;
            let text: Option<String> = r.get(7)?;
            let c = common_from(r, 8)?;
            let (mut attrs, prov, display) = c.base("message", &origin);
            put(&mut attrs, "role", Some(role.clone()));
            put(&mut attrs, "record_type", Some(record_type.clone()));
            put(&mut attrs, "message_uuid", uuid);
            put(&mut attrs, "model", model);
            let (source, slug) = source_for(&c.harness);
            let body = text.unwrap_or_default();
            let message = if body.is_empty() { format!("[{}] {}: <{}>", c.harness, role, record_type) } else { format!("[{}] {}: {}", c.harness, role, truncate(&body, 4000)) };
            Ok(Event { datetime: ts, timestamp_desc: timestamp_desc("Message Timestamp", &origin), message, source, source_long: format!("{slug}:message:{role}"), display_name: display, tags: vec![c.harness.clone(), role, origin], attributes: attrs, provenance: prov })
        })?;
        out.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }

    // Tool calls: one event at the start time; provenance is the tool_use block's record.
    {
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let clause = scope_clause(scope, &mut args);
        let sql = format!(
            "SELECT t.id, t.name, t.category, t.input, t.started_ts, t.ts_origin, t.is_error, {COMMON_COLS}
             FROM tool_calls t JOIN sessions s ON s.id = t.session_id JOIN stores st ON st.id = s.store_id JOIN evidence_roots r ON r.id = st.root_id
                  JOIN blocks b ON b.id = t.tool_use_block_id JOIN provenance p ON p.id = b.provenance_id JOIN source_files f ON f.id = p.source_file_id
             WHERE 1 = 1 {clause} ORDER BY t.id"
        );
        let mut stmt = case.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
            let name: String = r.get(1)?;
            let category: String = r.get(2)?;
            let input: String = r.get(3)?;
            let ts: Option<String> = r.get(4)?;
            let origin: String = r.get(5)?;
            let is_error: i64 = r.get(6)?;
            let c = common_from(r, 7)?;
            let (mut attrs, prov, display) = c.base("tool_call", &origin);
            put(&mut attrs, "tool_name", Some(name.clone()));
            put(&mut attrs, "tool_category", Some(category.clone()));
            put(&mut attrs, "tool_error", Some((is_error == 1).to_string()));
            let summary: String = serde_json::from_str::<serde_json::Value>(&input)
                .ok()
                .and_then(|v| ["command", "file_path", "url", "pattern", "description", "prompt"].iter().find_map(|k| v.get(k).and_then(|x| x.as_str()).map(String::from)))
                .unwrap_or_else(|| truncate(&input, 200));
            let (source, slug) = source_for(&c.harness);
            Ok(Event { datetime: ts, timestamp_desc: timestamp_desc("Tool Call Started", &origin), message: format!("[{}] tool {}: {}", c.harness, name, truncate(&summary, 500)), source, source_long: format!("{slug}:tool_call:{category}"), display_name: display, tags: vec![c.harness.clone(), "tool_call".to_string(), category, origin], attributes: attrs, provenance: prov })
        })?;
        out.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }

    // Observations: provenance comes from the tool call's tool_use block, the block, or the record.
    {
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let clause = scope_clause(scope, &mut args);
        let sql = format!(
            "SELECT o.id, o.kind, o.path, o.command, o.confidence, o.timestamp, o.ts_origin, {COMMON_COLS}
             FROM observations o JOIN sessions s ON s.id = o.session_id JOIN stores st ON st.id = s.store_id JOIN evidence_roots r ON r.id = st.root_id
                  JOIN provenance p ON p.id = COALESCE(o.derived_from_provenance_id,
                        (SELECT b.provenance_id FROM blocks b WHERE b.id = o.derived_from_block_id),
                        (SELECT b.provenance_id FROM tool_calls t JOIN blocks b ON b.id = t.tool_use_block_id WHERE t.id = o.derived_from_tool_call_id))
                  JOIN source_files f ON f.id = p.source_file_id
             WHERE 1 = 1 {clause} ORDER BY o.id"
        );
        let mut stmt = case.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
            let kind: String = r.get(1)?;
            let path: Option<String> = r.get(2)?;
            let command: Option<String> = r.get(3)?;
            let confidence: String = r.get(4)?;
            let ts: Option<String> = r.get(5)?;
            let origin: String = r.get(6)?;
            let c = common_from(r, 7)?;
            let (mut attrs, prov, display) = c.base("observation", &origin);
            put(&mut attrs, "observation_kind", Some(kind.clone()));
            put(&mut attrs, "path", path.clone());
            put(&mut attrs, "command", command.clone());
            put(&mut attrs, "confidence", Some(confidence.clone()));
            let what = command.clone().or(path.clone()).unwrap_or_default();
            let (source, slug) = source_for(&c.harness);
            Ok(Event { datetime: ts, timestamp_desc: timestamp_desc("Observation Time", &origin), message: format!("[{}] {}: {}", c.harness, kind, truncate(&what, 500)), source, source_long: format!("{slug}:observation:{kind}"), display_name: display, tags: vec![c.harness.clone(), "observation".to_string(), kind, confidence, origin], attributes: attrs, provenance: prov })
        })?;
        out.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }

    out.sort_by(|a, b| match (&a.datetime, &b.datetime) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    Ok(out)
}
```

- [ ] **Step 4: Implement the writers**

`crates/vem-case/src/export/timesketch.rs`:

```rust
//! Timesketch / Vestigo CSV and JSONL writers (spec §9; Vestigo `INPUT_FORMATS.md`).

use super::Event;
use crate::error::CaseError;
use std::collections::BTreeSet;
use std::io::Write;

const FIXED: [&str; 7] = ["datetime", "timestamp_desc", "message", "source", "source_long", "display_name", "tag"];

pub fn write_jsonl(events: &[Event], mut w: impl Write) -> Result<(), CaseError> {
    for e in events {
        let mut obj = serde_json::Map::new();
        obj.insert("datetime".into(), e.datetime.clone().map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
        obj.insert("timestamp_desc".into(), e.timestamp_desc.clone().into());
        obj.insert("message".into(), e.message.clone().into());
        obj.insert("source".into(), e.source.clone().into());
        obj.insert("source_long".into(), e.source_long.clone().into());
        obj.insert("display_name".into(), e.display_name.clone().into());
        obj.insert("tags".into(), serde_json::Value::Array(e.tags.iter().cloned().map(serde_json::Value::String).collect()));
        for (k, v) in &e.attributes {
            if !FIXED.contains(&k.as_str()) && k != "tags" {
                obj.insert(k.clone(), v.clone().into());
            }
        }
        serde_json::to_writer(&mut w, &serde_json::Value::Object(obj))?;
        w.write_all(b"\n")?;
    }
    Ok(())
}

pub fn write_csv(events: &[Event], w: impl Write) -> Result<(), CaseError> {
    let mut extra: BTreeSet<String> = BTreeSet::new();
    for e in events {
        for k in e.attributes.keys() {
            if !FIXED.contains(&k.as_str()) && k != "tags" {
                extra.insert(k.clone());
            }
        }
    }
    let mut wtr = csv::Writer::from_writer(w);
    let mut header: Vec<&str> = FIXED.to_vec();
    header.extend(extra.iter().map(String::as_str));
    wtr.write_record(&header).map_err(|e| CaseError::Export(e.to_string()))?;
    for e in events {
        let mut row: Vec<String> = vec![
            e.datetime.clone().unwrap_or_default(),
            e.timestamp_desc.clone(),
            e.message.clone(),
            e.source.clone(),
            e.source_long.clone(),
            e.display_name.clone(),
            e.tags.join("|"),
        ];
        for k in &extra {
            row.push(e.attributes.get(k).cloned().unwrap_or_default());
        }
        wtr.write_record(&row).map_err(|e| CaseError::Export(e.to_string()))?;
    }
    wtr.flush()?;
    Ok(())
}
```

Add `pub mod export;` to `crates/vem-case/src/lib.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p vem-case --test export_timesketch`
Expected: 4 passed.

- [ ] **Step 6: Commit**

```bash
git add crates/vem-case
git commit -m "feat(case): timesketch-compatible jsonl and csv export with provenance attributes

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 16: Vestigo Parquet export

**Files:**
- Create: `crates/vem-case/src/export/parquet.rs`
- Create: `crates/vem-case/tests/export_parquet.rs`
- Modify: `crates/vem-case/src/export/mod.rs`

**Interfaces:**
- Consumes: `Event`, `events()`.
- Produces: `vem_case::export::parquet::{write_parquet(case: &Case, events: &[Event], out: &Path) -> Result<ParquetReport>, ParquetReport { rows: usize, original_files: usize }, schema() -> arrow::datatypes::Schema}`. Footer metadata: `vestigo.format_version` = `"1"`, `vestigo.converter_name` = `"vem"`, `vestigo.converter_version` = `TOOL_VERSION`, `vestigo.original_files` = JSON array of `{name, sha256, size_bytes, path, mtime}` (one per distinct evidence file referenced by the events), `vestigo.converted_at` = now, `vestigo.row_counts` = `{"parsed": n, "skipped_malformed": 0, "skipped_by_time": 0}`.

- [ ] **Step 1: Write the failing test**

`crates/vem-case/tests/export_parquet.rs`:

```rust
mod common;

use common::*;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::parquet::write_parquet;
use vem_case::export::{events, Scope};
use vem_case::ingest::ingest;
use vem_case::Case;

#[test]
fn writes_vestigo_v1_schema_and_footer() {
    let tmp = tempfile::tempdir().unwrap();
    let mut case = Case::create(&tmp.path().join("c"), "n", None).unwrap();
    attach(&mut case, &fixture_root(), AttachOptions { label: "alice".into(), host: None, user: None, os: None, harness: None, retain: true }).unwrap();
    ingest(&mut case, None).unwrap();
    let ev = events(&case, &Scope::Case).unwrap();
    let out = tmp.path().join("case.parquet");
    let report = write_parquet(&case, &ev, &out).unwrap();
    assert_eq!(report.rows, ev.len());
    assert!(report.original_files >= 4, "S1, S0, subagent, orphan transcripts and history.jsonl");

    let builder = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&out).unwrap()).unwrap();
    let schema = builder.schema().clone();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(names, vec!["source_file", "file_hash", "byte_offset", "content_hash", "message", "timestamp", "timestamp_desc", "artifact", "artifact_long", "display_name", "tags", "attributes"]);
    use arrow::datatypes::{DataType, TimeUnit};
    assert_eq!(schema.field_with_name("byte_offset").unwrap().data_type(), &DataType::UInt64);
    assert_eq!(schema.field_with_name("timestamp").unwrap().data_type(), &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())));
    assert!(matches!(schema.field_with_name("tags").unwrap().data_type(), DataType::List(_)));
    assert!(matches!(schema.field_with_name("attributes").unwrap().data_type(), DataType::Map(_, _)));
    let kv = builder.metadata().file_metadata().key_value_metadata().cloned().unwrap_or_default();
    let get = |k: &str| kv.iter().find(|x| x.key == k).and_then(|x| x.value.clone()).unwrap_or_else(|| panic!("missing footer key {k}"));
    assert_eq!(get("vestigo.format_version"), "1");
    assert_eq!(get("vestigo.converter_name"), "vem");
    assert_eq!(get("vestigo.converter_version"), vem_case::TOOL_VERSION);
    let originals: Vec<serde_json::Value> = serde_json::from_str(&get("vestigo.original_files")).unwrap();
    assert_eq!(originals.len(), report.original_files);
    assert!(originals.iter().all(|o| o["name"].is_string() && o["sha256"].as_str().unwrap().len() == 64 && o["size_bytes"].is_number()));
    let total: usize = builder.build().unwrap().map(|b| b.unwrap().num_rows()).sum();
    assert_eq!(total, ev.len());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem-case --test export_parquet`
Expected: compile error, `parquet` module missing.

- [ ] **Step 3: Implement**

`crates/vem-case/src/export/parquet.rs`:

```rust
//! Vestigo interchange Parquet, format version 1 (verified against Vestigo's `parquet_format.py`).

use super::Event;
use crate::case::now;
use crate::error::CaseError;
use crate::{Case, TOOL_VERSION};
use arrow::array::{ArrayRef, ListBuilder, MapBuilder, MapFieldNames, StringBuilder, TimestampMillisecondBuilder, UInt64Builder};
use arrow::datatypes::{DataType, Field, Fields, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;
use parquet::file::metadata::KeyValue;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct ParquetReport {
    pub rows: usize,
    pub original_files: usize,
}

fn map_names() -> MapFieldNames {
    MapFieldNames { entry: "entries".to_string(), key: "key".to_string(), value: "value".to_string() }
}

pub fn schema() -> Schema {
    let entries = Field::new(
        "entries",
        DataType::Struct(Fields::from(vec![Field::new("key", DataType::Utf8, false), Field::new("value", DataType::Utf8, true)])),
        false,
    );
    Schema::new(vec![
        Field::new("source_file", DataType::Utf8, false),
        Field::new("file_hash", DataType::Utf8, false),
        Field::new("byte_offset", DataType::UInt64, false),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new("message", DataType::Utf8, false),
        Field::new("timestamp", DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())), true),
        Field::new("timestamp_desc", DataType::Utf8, false),
        Field::new("artifact", DataType::Utf8, false),
        Field::new("artifact_long", DataType::Utf8, false),
        Field::new("display_name", DataType::Utf8, false),
        Field::new("tags", DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))), false),
        Field::new("attributes", DataType::Map(Arc::new(entries), false), false),
    ])
}

fn millis(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|d| d.timestamp_millis())
}

fn arrow_err(e: arrow::error::ArrowError) -> CaseError {
    CaseError::Export(e.to_string())
}

fn parquet_err(e: parquet::errors::ParquetError) -> CaseError {
    CaseError::Export(e.to_string())
}

pub fn write_parquet(case: &Case, events: &[Event], out: &Path) -> Result<ParquetReport, CaseError> {
    let mut source_file = StringBuilder::new();
    let mut file_hash = StringBuilder::new();
    let mut byte_offset = UInt64Builder::new();
    let mut content_hash = StringBuilder::new();
    let mut message = StringBuilder::new();
    let mut timestamp = TimestampMillisecondBuilder::new().with_timezone("UTC");
    let mut timestamp_desc = StringBuilder::new();
    let mut artifact = StringBuilder::new();
    let mut artifact_long = StringBuilder::new();
    let mut display_name = StringBuilder::new();
    let mut tags = ListBuilder::new(StringBuilder::new());
    let mut attributes = MapBuilder::new(Some(map_names()), StringBuilder::new(), StringBuilder::new());
    let mut originals: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    for e in events {
        source_file.append_value(&e.provenance.source_file);
        file_hash.append_value(&e.provenance.file_sha256);
        byte_offset.append_value(e.provenance.byte_offset);
        content_hash.append_value(&e.provenance.content_sha256);
        message.append_value(&e.message);
        match e.datetime.as_deref().and_then(millis) {
            Some(ms) => timestamp.append_value(ms),
            None => timestamp.append_null(),
        }
        timestamp_desc.append_value(&e.timestamp_desc);
        artifact.append_value(&e.source);
        artifact_long.append_value(&e.source_long);
        display_name.append_value(&e.display_name);
        for t in &e.tags {
            tags.values().append_value(t);
        }
        tags.append(true);
        for (k, v) in &e.attributes {
            attributes.keys().append_value(k);
            attributes.values().append_value(v);
        }
        attributes.append(true).map_err(arrow_err)?;
        originals.entry(e.provenance.file_sha256.clone()).or_insert_with(|| {
            let name = Path::new(&e.provenance.source_file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| e.provenance.source_file.clone());
            serde_json::json!({ "name": name, "sha256": e.provenance.file_sha256, "size_bytes": e.provenance.file_size, "path": e.provenance.source_file, "mtime": e.provenance.file_mtime })
        });
    }

    let columns: Vec<ArrayRef> = vec![
        Arc::new(source_file.finish()),
        Arc::new(file_hash.finish()),
        Arc::new(byte_offset.finish()),
        Arc::new(content_hash.finish()),
        Arc::new(message.finish()),
        Arc::new(timestamp.finish()),
        Arc::new(timestamp_desc.finish()),
        Arc::new(artifact.finish()),
        Arc::new(artifact_long.finish()),
        Arc::new(display_name.finish()),
        Arc::new(tags.finish()),
        Arc::new(attributes.finish()),
    ];
    let schema = Arc::new(schema());
    let batch = RecordBatch::try_new(schema.clone(), columns).map_err(arrow_err)?;

    let original_files: Vec<serde_json::Value> = originals.into_values().collect();
    let metadata = vec![
        KeyValue::new("vestigo.format_version".to_string(), "1".to_string()),
        KeyValue::new("vestigo.converter_name".to_string(), "vem".to_string()),
        KeyValue::new("vestigo.converter_version".to_string(), TOOL_VERSION.to_string()),
        KeyValue::new("vestigo.original_files".to_string(), serde_json::to_string(&original_files)?),
        KeyValue::new("vestigo.converted_at".to_string(), now()),
        KeyValue::new("vestigo.row_counts".to_string(), serde_json::json!({ "parsed": events.len(), "skipped_malformed": 0, "skipped_by_time": 0 }).to_string()),
        KeyValue::new("vestigo.timezone_assumption".to_string(), "all timestamps stored UTC by vem; origin per row in timestamp_desc and attributes.ts_origin".to_string()),
    ];
    let props = WriterProperties::builder().set_key_value_metadata(Some(metadata)).build();
    let file = std::fs::File::create(out)?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).map_err(parquet_err)?;
    writer.write(&batch).map_err(parquet_err)?;
    writer.close().map_err(parquet_err)?;
    let _ = case;
    Ok(ParquetReport { rows: events.len(), original_files: original_files.len() })
}
```

Add `pub mod parquet;` to `crates/vem-case/src/export/mod.rs`. If `MapBuilder::append` in arrow 60 returns `()` rather than `Result`, drop the `.map_err(arrow_err)?`; if `RecordBatch::try_new` rejects the `tags` or `attributes` column because the builder's field nullability differs from `schema()`, change `schema()` to match the builder's `data_type()` (print `columns[10].data_type()` and `columns[11].data_type()` in a quick `dbg!`) rather than fighting the builder: the Vestigo reader compares pyarrow types, which treat list and map field names and nullability leniently, while the names `entries`/`key`/`value` are what pyarrow emits.

- [ ] **Step 4: Run the test**

Run: `cargo test -p vem-case --test export_parquet`
Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/vem-case
git commit -m "feat(case): vestigo interchange parquet export with footer provenance

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 17: CLI and end-to-end test

**Files:**
- Modify: `crates/vem/src/main.rs`
- Create: `crates/vem/src/commands.rs`
- Create: `crates/vem/tests/cli.rs`

**Interfaces:**
- Consumes: everything public in `vem_case`.
- Produces the `vem` binary with this surface:

```
vem case new <dir> --name <name> [--examiner <name>]
vem evidence add <case> <path> --label <label> [--host H] [--user U] [--os linux|macos|windows] [--harness claude-code|codex|cursor|cursor-ide] [--no-retain]
vem evidence list <case>
vem ingest <case> [--root <id>]
vem verify <case>
vem inventory <case>
vem sessions <case> [--root <id>] [--kind <kind>]
vem export <case> --format timesketch-jsonl|timesketch-csv|vestigo-parquet [--root <id>] [--session <id>] -o <file>
```
Global flag `--json` prints each command's report as JSON instead of text. Exit code 0 on success, 1 on any error (message on stderr), 2 when evidence is unrecognized (the child-directory hint is printed).

- [ ] **Step 1: Write the failing end-to-end test**

`crates/vem/tests/cli.rs`:

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/claude-code/basic").canonicalize().unwrap()
}

fn vem() -> Command {
    Command::cargo_bin("vem").unwrap()
}

#[test]
fn full_headless_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    let case = tmp.path().join("case1");
    let case_s = case.to_string_lossy().to_string();

    vem().args(["case", "new", &case_s, "--name", "Incident 42", "--examiner", "ex"]).assert().success().stdout(predicate::str::contains("Incident 42"));

    vem().args(["evidence", "add", &case_s, fixture().to_str().unwrap(), "--label", "alice .claude", "--host", "laptop", "--user", "alice", "--os", "linux"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-code"))
        .stdout(predicate::str::contains("claude:projects"))
        .stdout(predicate::str::contains("absent"));

    vem().args(["evidence", "list", &case_s]).assert().success().stdout(predicate::str::contains("alice .claude"));

    vem().args(["ingest", &case_s]).assert().success().stdout(predicate::str::contains("sessions: 4")).stdout(predicate::str::contains("anomalies: 4"));

    vem().args(["ingest", &case_s]).assert().success().stdout(predicate::str::contains("files_parsed: 0"));

    vem().args(["inventory", &case_s])
        .assert()
        .success()
        .stdout(predicate::str::contains("truncated_line"))
        .stdout(predicate::str::contains("missing_transcript"))
        .stdout(predicate::str::contains("claude:todos"));

    vem().args(["sessions", &case_s]).assert().success().stdout(predicate::str::contains("0f0f0f0f-0000-4000-8000-000000000001")).stdout(predicate::str::contains("resumed")).stdout(predicate::str::contains("subagent"));

    let sessions_json = vem().args(["--json", "sessions", &case_s]).output().unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&sessions_json.stdout).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 4);

    let jsonl = tmp.path().join("tl.jsonl");
    vem().args(["export", &case_s, "--format", "timesketch-jsonl", "-o", jsonl.to_str().unwrap()]).assert().success().stdout(predicate::str::contains("32"));
    let text = std::fs::read_to_string(&jsonl).unwrap();
    assert_eq!(text.lines().count(), 32);
    let first: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(first.get("timestamp_desc").is_some());

    let csv = tmp.path().join("tl.csv");
    vem().args(["export", &case_s, "--format", "timesketch-csv", "-o", csv.to_str().unwrap()]).assert().success();
    assert!(std::fs::read_to_string(&csv).unwrap().starts_with("datetime,timestamp_desc,message,"));

    let pq = tmp.path().join("tl.parquet");
    vem().args(["export", &case_s, "--format", "vestigo-parquet", "-o", pq.to_str().unwrap()]).assert().success();
    assert!(std::fs::metadata(&pq).unwrap().len() > 0);

    vem().args(["verify", &case_s]).assert().success().stdout(predicate::str::contains("drifted: 0")).stdout(predicate::str::contains("missing: 0"));

    let audits = vem().args(["--json", "inventory", &case_s]).output().unwrap();
    let inv: serde_json::Value = serde_json::from_slice(&audits.stdout).unwrap();
    assert!(inv["audit_log"].as_array().unwrap().iter().any(|a| a["action"] == "export"));
}

#[test]
fn unrecognized_evidence_exits_2_with_hint() {
    let tmp = tempfile::tempdir().unwrap();
    let case = tmp.path().join("case2");
    vem().args(["case", "new", case.to_str().unwrap(), "--name", "n"]).assert().success();
    let collection = tmp.path().join("collection");
    std::fs::create_dir_all(collection.join("unrelated")).unwrap();
    let dot_claude = collection.join(".claude");
    for entry in walkdir::WalkDir::new(fixture()) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(fixture()).unwrap();
        let t = dot_claude.join(rel);
        if entry.file_type().is_dir() { std::fs::create_dir_all(&t).unwrap(); } else { std::fs::create_dir_all(t.parent().unwrap()).unwrap(); std::fs::copy(entry.path(), &t).unwrap(); }
    }
    vem().args(["evidence", "add", case.to_str().unwrap(), collection.to_str().unwrap(), "--label", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(".claude"))
        .stderr(predicate::str::contains("claude-code"));
}
```

Add `walkdir.workspace = true` and `serde_json.workspace = true` under `[dev-dependencies]` in `crates/vem/Cargo.toml` (serde_json is already a normal dependency; walkdir must be added).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p vem --test cli`
Expected: the first test fails at `case new` (unknown subcommand).

- [ ] **Step 3: Implement the CLI**

`crates/vem/src/main.rs`:

```rust
//! vem: Vestigia Ex Machina. Forensic analysis of agentic AI harness traces.

mod commands;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "vem", version, about = "Forensic analysis of agentic AI harness traces")]
struct Cli {
    /// Print reports as JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Case management.
    Case {
        #[command(subcommand)]
        cmd: CaseCmd,
    },
    /// Evidence roots (collected harness directories).
    Evidence {
        #[command(subcommand)]
        cmd: EvidenceCmd,
    },
    /// Parse every unparsed file of every attached root.
    Ingest {
        case: PathBuf,
        #[arg(long)]
        root: Option<i64>,
    },
    /// Re-hash evidence and retained copies against the manifest.
    Verify { case: PathBuf },
    /// Roots, stores, absent stores, files, anomalies and audit log.
    Inventory { case: PathBuf },
    /// List sessions.
    Sessions {
        case: PathBuf,
        #[arg(long)]
        root: Option<i64>,
        #[arg(long)]
        kind: Option<String>,
    },
    /// Export a timeline.
    Export {
        case: PathBuf,
        #[arg(long, value_enum)]
        format: ExportFormat,
        #[arg(long)]
        root: Option<i64>,
        #[arg(long)]
        session: Option<i64>,
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum CaseCmd {
    /// Create a new case directory.
    New {
        dir: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        examiner: Option<String>,
    },
}

#[derive(Subcommand)]
enum EvidenceCmd {
    /// Attach a collected harness directory.
    Add {
        case: PathBuf,
        path: PathBuf,
        #[arg(long)]
        label: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        os: Option<String>,
        #[arg(long, value_enum)]
        harness: Option<HarnessArg>,
        #[arg(long)]
        no_retain: bool,
    },
    /// List attached roots.
    List { case: PathBuf },
}

#[derive(Clone, Copy, ValueEnum)]
enum HarnessArg {
    ClaudeCode,
    Codex,
    Cursor,
    CursorIde,
}

impl From<HarnessArg> for vem_core::model::Harness {
    fn from(h: HarnessArg) -> Self {
        match h {
            HarnessArg::ClaudeCode => Self::ClaudeCode,
            HarnessArg::Codex => Self::Codex,
            HarnessArg::Cursor => Self::Cursor,
            HarnessArg::CursorIde => Self::CursorIde,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ExportFormat {
    TimesketchJsonl,
    TimesketchCsv,
    VestigoParquet,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Case { cmd: CaseCmd::New { dir, name, examiner } } => commands::case_new(&dir, &name, examiner.as_deref(), cli.json),
        Command::Evidence { cmd: EvidenceCmd::Add { case, path, label, host, user, os, harness, no_retain } } => {
            commands::evidence_add(&case, &path, label, host, user, os, harness.map(Into::into), !no_retain, cli.json)
        }
        Command::Evidence { cmd: EvidenceCmd::List { case } } => commands::evidence_list(&case, cli.json),
        Command::Ingest { case, root } => commands::ingest(&case, root, cli.json),
        Command::Verify { case } => commands::verify(&case, cli.json),
        Command::Inventory { case } => commands::inventory(&case, cli.json),
        Command::Sessions { case, root, kind } => commands::sessions(&case, root, kind, cli.json),
        Command::Export { case, format, root, session, output } => commands::export(&case, format, root, session, &output, cli.json),
    };
    match result {
        Ok(()) => {}
        Err(vem_case::CaseError::Unrecognized { path, hint }) => {
            eprintln!("error: {} is not recognized as a harness directory{}", path.display(), hint);
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
```

`crates/vem/src/commands.rs`:

```rust
//! One function per subcommand. Each prints text or, with `--json`, the report as JSON.

use crate::ExportFormat;
use serde::Serialize;
use std::path::Path;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::{events, Scope};
use vem_case::query;
use vem_case::{Case, CaseError};
use vem_core::model::Harness;

fn emit<T: Serialize>(json: bool, value: &T, text: impl FnOnce(&T) -> String) -> Result<(), CaseError> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", text(value));
    }
    Ok(())
}

pub fn case_new(dir: &Path, name: &str, examiner: Option<&str>, json: bool) -> Result<(), CaseError> {
    let case = Case::create(dir, name, examiner)?;
    let info = case.info()?;
    emit(json, &info, |i| format!("created case {:?} at {} (examiner: {}, vem {})", i.name, dir.display(), i.examiner.as_deref().unwrap_or("-"), i.tool_version))
}

#[allow(clippy::too_many_arguments)]
pub fn evidence_add(case_dir: &Path, path: &Path, label: String, host: Option<String>, user: Option<String>, os: Option<String>, harness: Option<Harness>, retain: bool, json: bool) -> Result<(), CaseError> {
    let mut case = Case::open(case_dir)?;
    let report = attach(&mut case, path, AttachOptions { label, host, user, os, harness, retain })?;
    emit(json, &report, |r| {
        let mut s = format!("attached root {} as {} ({} files, {} bytes, {} unclaimed, retained: {})\n  evidence: {}\n", r.root_id, r.harness, r.file_count, r.total_bytes, r.unclaimed_files, retain, r.evidence.join("; "));
        for st in &r.stores {
            s.push_str(&format!("  store {} {} ({} files){}\n", st.id, st.kind, st.file_count, st.generation.as_ref().map(|g| format!(", generation {g}")).unwrap_or_default()));
        }
        if !r.absent.is_empty() {
            s.push_str(&format!("  absent: {}\n", r.absent.join(", ")));
        }
        if !r.unreadable.is_empty() {
            s.push_str(&format!("  unreadable: {}\n", r.unreadable.join(", ")));
        }
        s
    })
}

pub fn evidence_list(case_dir: &Path, json: bool) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let roots = query::roots(&case)?;
    emit(json, &roots, |rs| rs.iter().map(|r| format!("{}  {}  {}  {}  host={} user={} os={}", r.id, r.harness, r.label, r.path, r.host.as_deref().unwrap_or("-"), r.user.as_deref().unwrap_or("-"), r.os.as_deref().unwrap_or("-"))).collect::<Vec<_>>().join("\n"))
}

pub fn ingest(case_dir: &Path, root: Option<i64>, json: bool) -> Result<(), CaseError> {
    let mut case = Case::open(case_dir)?;
    let report = vem_case::ingest::ingest(&mut case, root)?;
    emit(json, &report, |r| {
        format!(
            "ingest done\n  files_parsed: {}\n  files_failed: {}\n  files_skipped: {}\n  sessions: {}\n  messages: {}\n  tool_calls: {}\n  observations: {}\n  anomalies: {}",
            r.files_parsed, r.files_failed, r.files_skipped, r.sessions, r.messages, r.tool_calls, r.observations, r.anomalies
        )
    })
}

pub fn verify(case_dir: &Path, json: bool) -> Result<(), CaseError> {
    let mut case = Case::open(case_dir)?;
    let report = vem_case::verify::verify(&mut case)?;
    emit(json, &report, |r| {
        format!(
            "verify done\n  files_checked: {}\n  drifted: {}{}\n  missing: {}{}\n  roots_unavailable: {}\n  blobs_checked: {}\n  blob_errors: {}",
            r.files_checked,
            r.drifted.len(),
            if r.drifted.is_empty() { String::new() } else { format!(" ({})", r.drifted.join(", ")) },
            r.missing.len(),
            if r.missing.is_empty() { String::new() } else { format!(" ({})", r.missing.join(", ")) },
            r.roots_unavailable.len(),
            r.blobs_checked,
            r.blob_errors.len()
        )
    })
}

#[derive(Serialize)]
struct Inventory {
    case: vem_case::CaseInfo,
    roots: Vec<RootInventory>,
    anomalies: Vec<query::AnomalyRow>,
    audit_log: Vec<AuditRow>,
}

#[derive(Serialize)]
struct RootInventory {
    root: query::RootRow,
    stores: Vec<query::StoreRow>,
    absent: Vec<String>,
    files: Vec<query::SourceFileRow>,
}

#[derive(Serialize)]
struct AuditRow {
    id: i64,
    ts: String,
    action: String,
    target: Option<String>,
    details: serde_json::Value,
}

fn audit_rows(case: &Case) -> Result<Vec<AuditRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, ts, action, target, details FROM audit_log ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        let d: String = r.get(4)?;
        Ok(AuditRow { id: r.get(0)?, ts: r.get(1)?, action: r.get(2)?, target: r.get(3)?, details: serde_json::from_str(&d).unwrap_or(serde_json::Value::Null) })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn inventory(case_dir: &Path, json: bool) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let mut roots = Vec::new();
    for root in query::roots(&case)? {
        roots.push(RootInventory { stores: query::stores(&case, root.id)?, absent: query::absent_stores(&case, root.id)?, files: query::source_files(&case, root.id)?, root });
    }
    let inv = Inventory { case: case.info()?, roots, anomalies: query::anomalies(&case, &query::AnomalyFilter::default())?, audit_log: audit_rows(&case)? };
    emit(json, &inv, |inv| {
        let mut s = format!("case {:?} (vem {})\n", inv.case.name, inv.case.tool_version);
        for r in &inv.roots {
            s.push_str(&format!("root {} {} {} ({})\n", r.root.id, r.root.harness, r.root.label, r.root.path));
            for st in &r.stores {
                s.push_str(&format!("  store {} {} {} [{}] files={}\n", st.id, st.kind, st.rel_path, st.status, st.file_count));
            }
            if !r.absent.is_empty() {
                s.push_str(&format!("  absent stores: {}\n", r.absent.join(", ")));
            }
            for f in &r.files {
                s.push_str(&format!("  file {} {} {} bytes sha256={} {}{}\n", f.id, f.rel_path, f.size, &f.sha256[..12], f.parse_status, f.parse_error.as_ref().map(|e| format!(" ({e})")).unwrap_or_default()));
            }
        }
        s.push_str(&format!("anomalies ({}):\n", inv.anomalies.len()));
        for a in &inv.anomalies {
            s.push_str(&format!("  [{}] {} {}{}\n", a.severity, a.kind, a.message, a.byte_offset.map(|o| format!(" @{o}")).unwrap_or_default()));
        }
        s.push_str(&format!("audit log ({} entries)\n", inv.audit_log.len()));
        s
    })
}

pub fn sessions(case_dir: &Path, root: Option<i64>, kind: Option<String>, json: bool) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let rows = query::sessions(&case, &query::SessionFilter { root_id: root, kind, ..Default::default() })?;
    emit(json, &rows, |rows| {
        rows.iter()
            .map(|s| {
                format!(
                    "{}  {}  {}  {}  {}..{}  msgs={} tools={} anomalies={} children={}  {}",
                    s.id, s.harness, s.kind, s.harness_session_id,
                    s.first_ts.as_deref().unwrap_or("-"), s.last_ts.as_deref().unwrap_or("-"),
                    s.message_count, s.tool_call_count, s.anomaly_count, s.child_count,
                    s.title.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[derive(Serialize)]
struct ExportReport {
    format: String,
    output: String,
    events: usize,
    sha256: String,
}

pub fn export(case_dir: &Path, format: ExportFormat, root: Option<i64>, session: Option<i64>, output: &Path, json: bool) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let scope = match (root, session) {
        (_, Some(s)) => Scope::Session(s),
        (Some(r), None) => Scope::Root(r),
        (None, None) => Scope::Case,
    };
    let ev = events(&case, &scope)?;
    let format_name = match format {
        ExportFormat::TimesketchJsonl => {
            let f = std::fs::File::create(output)?;
            vem_case::export::timesketch::write_jsonl(&ev, std::io::BufWriter::new(f))?;
            "timesketch-jsonl"
        }
        ExportFormat::TimesketchCsv => {
            let f = std::fs::File::create(output)?;
            vem_case::export::timesketch::write_csv(&ev, std::io::BufWriter::new(f))?;
            "timesketch-csv"
        }
        ExportFormat::VestigoParquet => {
            vem_case::export::parquet::write_parquet(&case, &ev, output)?;
            "vestigo-parquet"
        }
    };
    let (sha256, _) = vem_core::hash::sha256_file(output)?;
    let report = ExportReport { format: format_name.to_string(), output: output.display().to_string(), events: ev.len(), sha256: sha256.clone() };
    case.audit("export", Some(&report.output), serde_json::to_value(&report)?)?;
    emit(json, &report, |r| format!("exported {} events as {} to {}\n  sha256: {}", r.events, r.format, r.output, r.sha256))
}
```

- [ ] **Step 4: Run the end-to-end tests**

Run:
```bash
cargo test -p vem --test cli
```
Expected: 2 passed. Then the whole workspace:
```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: all tests pass, clippy clean. Fix any warning clippy reports before committing.

- [ ] **Step 5: Document the CLI in the README**

Append to `README.md`:

````markdown

## Headless workflow

```bash
vem case new ./case-42 --name "Incident 42" --examiner "A. Examiner"
vem evidence add ./case-42 /evidence/alice/.claude --label "alice .claude" --host laptop-1 --user alice --os linux
vem ingest ./case-42
vem inventory ./case-42
vem sessions ./case-42
vem export ./case-42 --format timesketch-jsonl -o ./case-42/exports/timeline.jsonl
vem export ./case-42 --format vestigo-parquet  -o ./case-42/exports/timeline.parquet
vem verify ./case-42
```

Evidence is opened read-only and never modified. Every file is hashed on attach and retained
in the case directory, so the case stays self-contained after the evidence is detached.
````

- [ ] **Step 6: Commit**

```bash
git add crates/vem README.md Cargo.lock
git commit -m "feat(cli): headless vem commands for case, evidence, ingest, verify, inventory, sessions and export

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Spec items deliberately deferred to later plans

- Spec §6.1 `paste-cache` and `uploads` as `paste_detected` / `upload_detected` observations, and spec §4 `secret_candidate` scanning over prompts and tool I/O: both belong with the data-exposure views in plan 2, which adds the conversation UI. In this plan those stores are inventoried, hashed and retained only.
- Spec §7 annotations: the table exists; the write API arrives with the UI in plan 2.
- Spec §8 (web UI), §6.2 (Codex), §6.3 (Cursor): plans 2, 3 and 4.

## Done criteria for this plan

- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
- The e2e test in Task 17 passes, which exercises attach, ingest, idempotent re-ingest, inventory, sessions, three export formats and verify on the fixture.
- No test or command creates, modifies or deletes anything under `fixtures/` (the fingerprint assertions in Tasks 11 and 12 enforce this).
- The next plan (web API and conversation UI) consumes only `vem_case::query`, `vem_case::export` and `vem_case::ingest`.
