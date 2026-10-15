#!/bin/sh
# Install the latest CodeDna release binary.
#
#   curl -fsSL https://raw.githubusercontent.com/crafteraadarsh/codedna/main/install.sh | sh
#
# Environment:
#   CODEDNA_VERSION      release tag to install (default: latest), e.g. v0.2.0
#   CODEDNA_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
#
# The archive's SHA-256 checksum is verified before anything is installed.
# Nothing is run with sudo.

set -eu

REPO="crafteraadarsh/codedna"
VERSION="${CODEDNA_VERSION:-latest}"
INSTALL_DIR="${CODEDNA_INSTALL_DIR:-$HOME/.local/bin}"

err() {
    echo "codedna install: $*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || err "needs '$1' but it is not installed"
}

need curl
need tar
need uname

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux) os_part="unknown-linux-musl" ;;
    Darwin) os_part="apple-darwin" ;;
    *) err "unsupported OS '$os'. On Windows, use install.ps1 or download from https://github.com/$REPO/releases" ;;
esac
case "$arch" in
    x86_64 | amd64) arch_part="x86_64" ;;
    arm64 | aarch64) arch_part="aarch64" ;;
    *) err "unsupported CPU '$arch'. Build from source with: cargo install --git https://github.com/$REPO" ;;
esac

target="$arch_part-$os_part"
archive="codedna-$target.tar.gz"
if [ "$VERSION" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
else
    base="https://github.com/$REPO/releases/download/$VERSION"
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    err "needs 'sha256sum' or 'shasum' to verify the download"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Downloading $archive ($VERSION)..."
curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp/$archive" "$base/$archive" ||
    err "download failed: $base/$archive"
curl -fsSL --proto '=https' --tlsv1.2 -o "$tmp/$archive.sha256" "$base/$archive.sha256" ||
    err "checksum download failed"

expected="$(cut -d' ' -f1 <"$tmp/$archive.sha256")"
actual="$(sha "$tmp/$archive")"
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
    err "checksum mismatch for $archive; refusing to install"
fi

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$INSTALL_DIR"
install -m 755 "$tmp/codedna-$target/codedna" "$INSTALL_DIR/codedna" 2>/dev/null || {
    cp "$tmp/codedna-$target/codedna" "$INSTALL_DIR/codedna"
    chmod 755 "$INSTALL_DIR/codedna"
}

echo "Installed $("$INSTALL_DIR/codedna" --version) to $INSTALL_DIR/codedna"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add $INSTALL_DIR to your PATH to run 'codedna' from anywhere." ;;
esac
