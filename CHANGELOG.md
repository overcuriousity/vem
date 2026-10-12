# Changelog

## v0.4.0 (unreleased: tagged after the real-corpus QA gate)

### Added
- Cursor adapters. Harness `cursor` (a collected `.cursor` directory: agent transcripts and, as
  **experimental**, the `chats/*/*/store.db` chat store) and harness `cursor-ide` (Cursor's IDE
  application directory: the `cursorDiskKV` chat store in `state.vscdb`, workspace links and
  `User/History/` local history as `local_history` sessions with before/after diffs).
- Evidence SQLite is opened read-only and immutable. A database with a `-wal` file is read from a
  private WAL-applied copy (`wal_applied` anomaly, locator suffix ` +wal`), so the newest chats are not lost.
- Case-wide `cursor:composerId` identity claims link sessions across the two Cursor roots.
- `provenance.locator`: a record stored as a SQLite row or blob is addressed by a locator and its bytes
  are a verified content blob. Exports carry `evidence_locator` (Timesketch) and a nullable `locator`
  column (Vestigo Parquet) for such records.
- New anomaly kinds `unknown_field`, `unreferenced_record`, `wal_applied`; new session kind `local_history`.
- GUI: Cursor sessions render one card per turn with tool results inside their call chip; the
  provenance tab shows the locator; a binary record is shown as hex.

### Fixed
- The Sessions page harness filter sent `claude_code` / `cursor_ide`, which matched nothing; it now
  sends `claude-code`, `codex`, `cursor`, `cursor-ide`.

### Upgrade and downgrade
- **The case schema is now v2 (ADR-1).** v0.4.0 opens v0.3.0 cases and migrates them by adding the
  nullable `provenance.locator` column. v0.3.0 refuses a v2 case. To open a migrated case with v0.3.0,
  it must hold no Cursor roots (all locators NULL); then set `PRAGMA user_version = 1` on its
  `case.db` with the `sqlite3` CLI (v1 code ignores the extra column). This is manual and not
  supported by `vem`; work on a copy of the case.
