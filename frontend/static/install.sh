#!/usr/bin/env bash
# ololo CLI installer — Linux & macOS.
#
#   curl -fsSL https://ololo.dev/install.sh | bash
#
# Install a specific version (default: latest release):
#   curl -fsSL https://ololo.dev/install.sh | bash -s -- 0.1.0
#   OLOLO_VERSION=0.1.0 curl -fsSL https://ololo.dev/install.sh | bash
#
# Install directory (default: /usr/local/bin if writable, else ~/.local/bin):
#   OLOLO_INSTALL_DIR=~/bin curl -fsSL https://ololo.dev/install.sh | bash

set -euo pipefail

REPO="ololo-dev/ololo"
VERSION="${1:-${OLOLO_VERSION:-}}"

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar  >/dev/null 2>&1 || fail "tar is required"

# --- Detect platform ---------------------------------------------------------
os=$(uname -s)
arch=$(uname -m)

case "$arch" in
  x86_64|amd64)  arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) fail "unsupported architecture: $arch (supported: x86_64, aarch64)" ;;
esac

case "$os" in
  Linux)  artifact="ololo-linux-${arch}.tar.gz" ;;
  Darwin) artifact="ololo-macos-${arch}.tar.gz" ;;
  *) fail "unsupported OS: $os (use install.ps1 on Windows)" ;;
esac

# --- Resolve download URL ----------------------------------------------------
if [ -n "$VERSION" ]; then
  VERSION="${VERSION#v}"
  url="https://github.com/${REPO}/releases/download/v${VERSION}/${artifact}"
  say "Installing ololo v${VERSION} (${os}/${arch})"
else
  url="https://github.com/${REPO}/releases/latest/download/${artifact}"
  say "Installing ololo latest release (${os}/${arch})"
fi

# --- Pick install dir --------------------------------------------------------
if [ -n "${OLOLO_INSTALL_DIR:-}" ]; then
  bin_dir="$OLOLO_INSTALL_DIR"
elif [ -w /usr/local/bin ]; then
  bin_dir=/usr/local/bin
else
  bin_dir="$HOME/.local/bin"
fi
mkdir -p "$bin_dir"

# --- Download & install ------------------------------------------------------
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "Downloading ${url}"
curl -fSL --progress-bar "$url" -o "$tmp/$artifact" \
  || fail "download failed — check the version exists: https://github.com/${REPO}/releases"

tar -xzf "$tmp/$artifact" -C "$tmp"
[ -f "$tmp/ololo" ] || fail "archive did not contain the ololo binary"

install -m 755 "$tmp/ololo" "$bin_dir/ololo"
say "Installed to ${bin_dir}/ololo"

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *)
    printf '\033[1;33mnote:\033[0m %s is not in your PATH. Add it, e.g.:\n' "$bin_dir"
    printf '  export PATH="%s:$PATH"\n' "$bin_dir"
    ;;
esac

say "Done. Run 'ololo login' to get started."
