#!/usr/bin/env bash
# Builds the single self-contained vem binary: frontend first (embedded by rust-embed), then cargo.
# Needs Node/npm and network once for `npm ci`; the resulting binary needs neither.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
(cd frontend && npm ci && npm run typecheck && npm test && npm run build)
cargo build --release -p vem "$@"
bin=target/release/vem
[ -f target/"${CARGO_BUILD_TARGET:-}"/release/vem ] && bin=target/"${CARGO_BUILD_TARGET:-}"/release/vem
ls -l "$bin"
sha256sum "$bin"
