# Vestigia Ex Machina

Forensic analysis of the on-disk traces left by agentic AI coding harnesses
(Claude Code, Codex CLI, Cursor). Attach a collected harness directory to a case,
ingest it, browse sessions with per-record provenance, export Timesketch and
Vestigo timelines.

Design: `docs/superpowers/specs/2026-10-08-vem-design.md`.

## Install

Linux and macOS (x86_64, arm64), into `~/.local/bin`, checksum-verified:

```bash
curl -fsSL https://raw.githubusercontent.com/overcuriousity/vem/main/install.sh | sh
```

Pin a version with `VEM_VERSION=v0.1.0`, take the latest `main` build with
`VEM_VERSION=nightly`, change the target with `VEM_INSTALL_DIR=/path`. Windows and manual
downloads: [releases](https://github.com/overcuriousity/vem/releases). Every release asset has
a `.sha256` next to it and a combined `SHA256SUMS`.

Build from source: `cargo build --release -p vem` (Rust 1.85+).

## Releases

Every merge to `main` is built for Linux (static musl), macOS and Windows. If the workspace
version in `Cargo.toml` has no `v<version>` release yet, that build becomes the release;
otherwise it replaces the rolling `nightly` prerelease. To cut a release, bump the version in
`Cargo.toml` in a PR and merge it.


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
