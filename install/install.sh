#!/usr/bin/env sh
# witslog installer for Linux/macOS (FR-P8-002).
# Detects OS/arch, downloads the matching release asset from GitHub Releases,
# verifies its checksum, and places the binary on PATH.
#
# Usage: curl -fsSL https://.../install.sh | sh
# Env overrides: WITSLOG_VERSION (default: latest), WITSLOG_INSTALL_DIR (default: ~/.local/bin)

set -eu

REPO="all-wits/witslog"
VERSION="${WITSLOG_VERSION:-latest}"
INSTALL_DIR="${WITSLOG_INSTALL_DIR:-$HOME/.local/bin}"

os() {
  case "$(uname -s)" in
    Linux) echo "linux" ;;
    Darwin) echo "macos" ;;
    *) echo "unsupported"; ;;
  esac
}

arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x86_64" ;;
    arm64|aarch64) echo "aarch64" ;;
    *) echo "unsupported" ;;
  esac
}

OS="$(os)"
ARCH="$(arch)"

if [ "$OS" = "unsupported" ] || [ "$ARCH" = "unsupported" ]; then
  echo "error: no prebuilt witslog binary for this OS/arch ($(uname -s)/$(uname -m))." >&2
  echo "       install via cargo instead: cargo install witslog-cli" >&2
  exit 1
fi

ASSET="witslog-${OS}-${ARCH}.tar.gz"

if [ "$VERSION" = "latest" ]; then
  URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
  CHECKSUM_URL="https://github.com/${REPO}/releases/latest/download/${ASSET}.sha256"
else
  URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
  CHECKSUM_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}.sha256"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "Downloading ${ASSET} (${VERSION})..."
curl -fsSL "$URL" -o "$TMP_DIR/$ASSET"
curl -fsSL "$CHECKSUM_URL" -o "$TMP_DIR/$ASSET.sha256"

echo "Verifying checksum..."
( cd "$TMP_DIR" && sha256sum -c "$ASSET.sha256" ) || {
  echo "error: checksum verification failed, aborting install." >&2
  exit 1
}

tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"

mkdir -p "$INSTALL_DIR"
install -m 755 "$TMP_DIR/witslog" "$INSTALL_DIR/witslog"

echo "✓ witslog installed to $INSTALL_DIR/witslog"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "note: add $INSTALL_DIR to your PATH to use 'witslog' directly." ;;
esac

"$INSTALL_DIR/witslog" --version
