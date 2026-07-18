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
  *)
    # Pick the rc file the user's interactive shell actually reads. $SHELL is
    # the login shell, not necessarily $0 here (this script is piped into `sh`).
    # fish uses its own config file/syntax - `export PATH=...` is meaningless
    # there and .bashrc/.zshrc/.profile are never sourced by it.
    case "${SHELL:-}" in
      */fish)
        RC_FILE="$HOME/.config/fish/config.fish"
        LINE="set -gx PATH \"$INSTALL_DIR\" \$PATH"
        ;;
      */zsh)
        RC_FILE="$HOME/.zshrc"
        LINE="export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
      */bash)
        RC_FILE="$HOME/.bashrc"
        LINE="export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
      *)
        RC_FILE="$HOME/.profile"
        LINE="export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
    esac
    if [ -f "$RC_FILE" ] && grep -qF "$INSTALL_DIR" "$RC_FILE" 2>/dev/null; then
      : # already added on a previous install run
    else
      mkdir -p "$(dirname "$RC_FILE")"
      echo "" >> "$RC_FILE"
      echo "# added by witslog installer" >> "$RC_FILE"
      echo "$LINE" >> "$RC_FILE"
      echo "✓ added $INSTALL_DIR to PATH in $RC_FILE"
    fi
    # Update the current (piped-into-sh) session too, so the --version check
    # below works; a NEW interactive shell picks it up from $RC_FILE.
    export PATH="$INSTALL_DIR:$PATH"
    echo "note: restart your terminal (or run 'source $RC_FILE') for 'witslog' to work in new shells."
    ;;
esac

"$INSTALL_DIR/witslog" --version
