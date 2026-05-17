#!/usr/bin/env sh
# Installer for jetemail-cli on macOS and Linux.
#
# Usage:
#   curl -fsSL https://github.com/jetemail/jetemail-cli/releases/latest/download/install.sh | sh
#
# Honored env vars:
#   JETEMAIL_INSTALL_DIR  install location (default: $HOME/.local/bin)
#   JETEMAIL_VERSION      pin to a specific version, e.g. v0.1.2 (default: latest)
set -eu

REPO="jetemail/jetemail-cli"
BIN="jetemail"
INSTALL_DIR="${JETEMAIL_INSTALL_DIR:-$HOME/.local/bin}"

err() { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[32m==>\033[0m %s\n' "$1"; }

case "$(uname -s)" in
    Darwin)
        case "$(uname -m)" in
            arm64|aarch64) target="aarch64-apple-darwin" ;;
            *) err "unsupported macOS arch: $(uname -m)" ;;
        esac ;;
    Linux)
        case "$(uname -m)" in
            x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
            aarch64|arm64) target="aarch64-unknown-linux-gnu" ;;
            *) err "unsupported Linux arch: $(uname -m)" ;;
        esac ;;
    *) err "unsupported OS: $(uname -s) — use the PowerShell installer on Windows" ;;
esac

if [ -n "${JETEMAIL_VERSION:-}" ]; then
    tag="$JETEMAIL_VERSION"
else
    info "Looking up latest release"
    tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
    [ -n "$tag" ] || err "could not determine latest release"
fi
version="${tag#v}"

asset="${BIN}-${version}-${target}"
url="https://github.com/${REPO}/releases/download/${tag}/${asset}"

info "Downloading $asset"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/$BIN" || err "download failed: $url"
chmod +x "$tmp/$BIN"

mkdir -p "$INSTALL_DIR"
mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"
info "Installed to $INSTALL_DIR/$BIN"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        printf '\n\033[33mnote:\033[0m %s is not on your PATH.\n' "$INSTALL_DIR"
        printf '  Add this to your shell rc (e.g. ~/.zshrc, ~/.bashrc):\n'
        printf '    export PATH="%s:$PATH"\n\n' "$INSTALL_DIR"
        ;;
esac

"$INSTALL_DIR/$BIN" --version 2>/dev/null || true
