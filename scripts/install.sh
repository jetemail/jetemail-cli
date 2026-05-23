#!/usr/bin/env sh
# Installer for jetemail-cli on macOS and Linux.
#
# Usage:
#   curl -fsSL https://github.com/jetemail/jetemail-cli/releases/latest/download/install.sh | sh
#
# Honored env vars:
#   JETEMAIL_INSTALL_DIR     install location (default: $HOME/.local/bin)
#   JETEMAIL_VERSION         pin to a specific version, e.g. v0.1.2 (default: latest)
#   JETEMAIL_NO_MODIFY_PATH  set to 1 to skip editing shell rc files
#
# Flags (pass via `sh -s -- <flag>`):
#   --no-modify-path         same as JETEMAIL_NO_MODIFY_PATH=1
set -eu

REPO="jetemail/jetemail-cli"
BIN="jetemail"
INSTALL_DIR="${JETEMAIL_INSTALL_DIR:-$HOME/.local/bin}"
NO_MODIFY_PATH="${JETEMAIL_NO_MODIFY_PATH:-0}"

for arg in "$@"; do
    case "$arg" in
        --no-modify-path) NO_MODIFY_PATH=1 ;;
    esac
done

err() { printf '\033[31merror:\033[0m %s\n' "$1" >&2; exit 1; }
info() { printf '\033[32m==>\033[0m %s\n' "$1"; }
warn() { printf '\033[33mnote:\033[0m %s\n' "$1"; }

# Reject an INSTALL_DIR containing shell metacharacters or newlines before it is
# ever interpolated into a shell-rc PATH line (otherwise a hostile value would
# execute in every future login shell).
case "$INSTALL_DIR" in
    *'$'* | *'`'* | *'"'* | *"'"* | *';'* | *'&'* | *'|'* | *'<'* | *'>'* | *'('* | *')'* | *'
'*)
        err "JETEMAIL_INSTALL_DIR contains unsafe characters: $INSTALL_DIR" ;;
esac

# Compute the SHA-256 of $1 using whichever tool is present (Linux: sha256sum,
# macOS: shasum). Prints the bare hex digest.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        err "no sha256 tool (sha256sum or shasum) found to verify the download"
    fi
}

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

# Validate the tag is a plain semver before interpolating it into URLs — a
# poisoned API response or hostile JETEMAIL_VERSION can't redirect the download.
echo "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z.-]+)?$' \
    || err "unexpected version tag: $tag"
version="${tag#v}"

asset="${BIN}-${version}-${target}"
url="https://github.com/${REPO}/releases/download/${tag}/${asset}"
sums_url="https://github.com/${REPO}/releases/download/${tag}/SHA256SUMS"

info "Downloading $asset"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/$BIN" || err "download failed: $url"

# Verify the binary against the release's published SHA256SUMS before trusting
# it — fail closed if the checksum file is missing, omits this asset, or differs.
curl -fsSL "$sums_url" -o "$tmp/SHA256SUMS" \
    || err "could not fetch checksums: $sums_url"
expected=$(awk -v f="$asset" '$2==f || $2=="*"f {print $1; exit}' "$tmp/SHA256SUMS")
[ -n "$expected" ] || err "no checksum listed for $asset in SHA256SUMS"
actual=$(sha256_of "$tmp/$BIN")
[ "$expected" = "$actual" ] \
    || err "checksum mismatch for $asset (expected $expected, got $actual)"
info "Verified SHA-256 checksum"

chmod +x "$tmp/$BIN"

mkdir -p "$INSTALL_DIR"
mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"
info "Installed to $INSTALL_DIR/$BIN"

case ":$PATH:" in
    *":$INSTALL_DIR:"*)
        # Already on PATH for the current shell — nothing to do.
        ;;
    *)
        # Check whether a fresh login shell would already pick up INSTALL_DIR.
        # Ubuntu/Debian/Fedora ship a default ~/.profile that adds
        # $HOME/.local/bin to PATH when the directory exists, so a freshly
        # created ~/.local/bin only fails to register in the *current* shell.
        already_configured=0
        for rc in \
            "$HOME/.profile" \
            "$HOME/.bash_profile" \
            "$HOME/.bashrc" \
            "$HOME/.zprofile" \
            "$HOME/.zshrc" \
            "$HOME/.config/fish/config.fish"; do
            [ -f "$rc" ] || continue
            if grep -qF "$INSTALL_DIR" "$rc" 2>/dev/null; then
                already_configured=1; break
            fi
            if [ "$INSTALL_DIR" = "$HOME/.local/bin" ] \
                && grep -qF '.local/bin' "$rc" 2>/dev/null; then
                already_configured=1; break
            fi
        done

        if [ "$already_configured" = 1 ]; then
            printf '\n'
            warn "$INSTALL_DIR is not on PATH in this shell, but your shell config already references it."
            printf '  Open a new terminal (or run: exec "$SHELL" -l) to pick it up.\n\n'
        elif [ "$NO_MODIFY_PATH" = 1 ]; then
            printf '\n'
            warn "$INSTALL_DIR is not on your PATH."
            printf '  Add this to your shell rc:\n'
            printf '    export PATH="%s:$PATH"\n\n' "$INSTALL_DIR"
        else
            shell_name=$(basename "${SHELL:-}")
            case "$shell_name" in
                bash)
                    case "$(uname -s)" in
                        Darwin) rc="$HOME/.bash_profile" ;;
                        *)      rc="$HOME/.bashrc" ;;
                    esac ;;
                zsh)  rc="$HOME/.zshrc" ;;
                fish) rc="$HOME/.config/fish/config.fish" ;;
                *)    rc="" ;;
            esac

            if [ -z "$rc" ]; then
                printf '\n'
                warn "$INSTALL_DIR is not on your PATH; shell '${shell_name:-unknown}' was not recognized."
                printf '  Add this to your shell rc:\n'
                printf '    export PATH="%s:$PATH"\n\n' "$INSTALL_DIR"
            else
                mkdir -p "$(dirname "$rc")"
                touch "$rc"
                if grep -qF '# >>> jetemail PATH >>>' "$rc" 2>/dev/null; then
                    info "PATH entry already present in $rc"
                else
                    if [ "$shell_name" = "fish" ]; then
                        {
                            printf '\n# >>> jetemail PATH >>>\n'
                            printf 'if not contains "%s" $PATH\n    set -gx PATH "%s" $PATH\nend\n' "$INSTALL_DIR" "$INSTALL_DIR"
                            printf '# <<< jetemail PATH <<<\n'
                        } >> "$rc"
                    else
                        {
                            printf '\n# >>> jetemail PATH >>>\n'
                            printf 'case ":$PATH:" in *":%s:"*) ;; *) export PATH="%s:$PATH" ;; esac\n' "$INSTALL_DIR" "$INSTALL_DIR"
                            printf '# <<< jetemail PATH <<<\n'
                        } >> "$rc"
                    fi
                    info "Added $INSTALL_DIR to PATH in $rc"
                fi
                printf '  Run: source %s   (or open a new terminal) to use jetemail now.\n' "$rc"
                printf '  Skip rc edits with --no-modify-path or JETEMAIL_NO_MODIFY_PATH=1.\n\n'
            fi
        fi
        ;;
esac

"$INSTALL_DIR/$BIN" --version 2>/dev/null || true
