# Vestigia Ex Machina

Forensic analysis of the on-disk traces left by agentic AI coding harnesses
(Claude Code, Codex CLI, Cursor). Attach a collected harness directory to a case,
ingest it, browse sessions with per-record provenance, export Timesketch and
Vestigo timelines.

Design: `docs/superpowers/specs/2026-10-08-vem-design.md`.

Build: `cargo build --workspace` (Rust 1.85+).


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
