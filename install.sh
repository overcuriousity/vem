#!/bin/sh
# vem installer for Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/overcuriousity/vem/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/overcuriousity/vem/main/install.sh | VEM_VERSION=v0.2.0 sh
#
# Environment:
#   VEM_VERSION       release tag to install, e.g. v0.1.0 or nightly (default: latest release)
#   VEM_INSTALL_DIR   target directory (default: $HOME/.local/bin)
#   VEM_DOWNLOAD_BASE override the download URL prefix (testing and mirrors)
#
# The archive is checked against its published SHA-256 before anything is installed.
# Nothing is written outside VEM_INSTALL_DIR and no root access is needed.
set -eu

repo="overcuriousity/vem"
version="${VEM_VERSION:-latest}"
install_dir="${VEM_INSTALL_DIR:-$HOME/.local/bin}"

err() { echo "vem-install: $*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"; }
need curl
need tar
need uname
need mktemp

case "$(uname -s)" in
  Linux) os="unknown-linux-musl" ;;
  Darwin) os="apple-darwin" ;;
  *) err "unsupported OS $(uname -s); on Windows download the .zip from https://github.com/$repo/releases" ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  aarch64 | arm64) arch="aarch64" ;;
  *) err "unsupported architecture $(uname -m)" ;;
esac

# An x86_64 shell under Rosetta on Apple silicon still gets the native build.
if [ "$os" = "apple-darwin" ] && [ "$arch" = "x86_64" ] \
  && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
  arch="aarch64"
fi

target="$arch-$os"
asset="vem-$target.tar.gz"

if [ -n "${VEM_DOWNLOAD_BASE:-}" ]; then
  base="$VEM_DOWNLOAD_BASE"
elif [ "$version" = "latest" ]; then
  base="https://github.com/$repo/releases/latest/download"
else
  base="https://github.com/$repo/releases/download/$version"
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  err "need sha256sum or shasum to verify the download"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'rm -rf "$tmp"; exit 130' INT TERM

echo "vem-install: downloading $asset ($version)"
curl -fsSL --proto '=https,file' --proto-redir '=https' "$base/$asset" -o "$tmp/$asset" \
  || err "download failed: $base/$asset"
curl -fsSL --proto '=https,file' --proto-redir '=https' "$base/$asset.sha256" -o "$tmp/$asset.sha256" \
  || err "checksum download failed: $base/$asset.sha256"

expected="$(cut -d' ' -f1 <"$tmp/$asset.sha256")"
actual="$(sha256 "$tmp/$asset")"
[ -n "$expected" ] && [ "$expected" = "$actual" ] \
  || err "checksum mismatch for $asset (expected $expected, got $actual)"

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$install_dir"
cp "$tmp/vem-$target/vem" "$install_dir/vem.tmp"
chmod 755 "$install_dir/vem.tmp"
mv -f "$install_dir/vem.tmp" "$install_dir/vem"

echo "vem-install: installed $("$install_dir/vem" --version) to $install_dir/vem"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "vem-install: $install_dir is not on your PATH; add it, e.g. export PATH=\"$install_dir:\$PATH\"" ;;
esac
