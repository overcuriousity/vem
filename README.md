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

Settings go to `sh`, after the pipe:

```bash
curl -fsSL https://raw.githubusercontent.com/overcuriousity/vem/main/install.sh | VEM_VERSION=v0.1.0 sh
```

Pin a version with `VEM_VERSION=v0.1.0`, take the latest `main` build with
`VEM_VERSION=nightly`, change the target with `VEM_INSTALL_DIR=/path`. Windows and manual
downloads: [releases](https://github.com/overcuriousity/vem/releases). Every release asset has
a `.sha256` next to it and a combined `SHA256SUMS`.

Build from source: `scripts/build-release.sh` (Rust 1.85+, Node 24); see
[Building the single binary](#building-the-single-binary-air-gapped-use).

## Launch the GUI

The GUI is a local viewer built into the `vem` binary. Cases and evidence are set up on the CLI,
then `vem serve` opens the case in your default browser:

```bash
vem case new ./case-42 --name "Incident 42" --examiner "A. Examiner"
vem evidence add ./case-42 /evidence/alice/.claude --label "alice .claude"
vem ingest ./case-42
vem serve ./case-42            # prints http://127.0.0.1:8787/ and opens it; --no-open to skip
```

The server listens on 127.0.0.1 only. Stop it with Ctrl-C. See [Conversation viewer](#conversation-viewer).

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

## Conversation viewer

```bash
vem serve ./case-42            # http://127.0.0.1:8787/ ; --port to change, 0 picks a free port
vem serve ./case-42 --no-open  # print the URL without opening a browser
```

The server binds 127.0.0.1 only and answers only to `Host: 127.0.0.1:<port>` or `localhost:<port>`.
It reads the case and writes only two things, both recorded in the audit log: examiner annotations
(tag, bookmark, note) and exports into `<case>/exports/`. Attaching, ingesting and verifying stay on
the CLI. Nothing is fetched from the network: the UI is embedded in the binary.

Pages: case home (roots, stores, absent sub-stores, ingest status, inventory with hashes, anomalies),
sessions, the session view (conversation, tool-call chips, subagent tree, detail drawer with the
record, its verified raw bytes, provenance, observations, identity claims and annotations), activity
(commands, file operations with before/after diffs, indicators), search, export and the audit log.
Origin badges: green stored, amber inferred or derived, grey absent, blue local clock.

## Building the single binary (air-gapped use)

```bash
scripts/build-release.sh       # npm ci + vite build + cargo build --release -p vem
```

The result, `target/release/vem`, needs no Node, no network and no other files on the examiner
workstation. Only the build machine needs Node 24 / npm (and network once, for `npm ci`). A release
build without a built frontend fails on purpose; `VEM_ALLOW_NO_FRONTEND=1` builds a CLI-and-API-only
binary. For UI development run `vem serve` and `cd frontend && npm run dev` (Vite proxies `/api`).

## Derived indicators

Ingest derives `paste_detected` (prompt history, including large pastes stored in `paste-cache/`),
`upload_detected` (`uploads/<session>/`) and `secret_candidate` observations. Secret candidates come
from a fixed, versioned rule set (AWS, GitHub, GitLab, Slack, Anthropic, OpenAI and Google keys, private
keys, JWTs, and a low-confidence generic `password=` rule) scanned over prompts, tool inputs, tool results
and pasted content. The UI masks matches until revealed. A paste-cache file that no prompt references is
reported as an `orphaned_file` anomaly. Cases ingested by an earlier vem version do not gain these
derivations: ingest the evidence into a new case.
