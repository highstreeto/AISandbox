#!/bin/sh
# Installs (or updates) the AI Sandbox app `sandbox` on Linux, macOS and WSL.
#
#   curl -fsSL https://github.com/highstreeto/AISandbox/releases/latest/download/install.sh | sh
#
# SANDBOX_VERSION=v1.2.3   install that release instead of the newest one
# SANDBOX_INSTALL_DIR=...  install there instead of ~/.local/bin
set -eu

repo_url="https://github.com/highstreeto/AISandbox/releases"
install_dir="${SANDBOX_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) os=unknown-linux-musl ;;
    Darwin) os=apple-darwin ;;
    *) fail "unsupported system $(uname -s); on Windows use install.ps1" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) fail "unsupported CPU $(uname -m)" ;;
esac
target="$arch-$os"

if [ -n "${SANDBOX_VERSION:-}" ]; then
    case "$SANDBOX_VERSION" in v*) tag=$SANDBOX_VERSION ;; *) tag=v$SANDBOX_VERSION ;; esac
    base="$repo_url/download/$tag"
else
    base="$repo_url/latest/download"
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -O "$2" "$1"; }
else
    fail "needs curl or wget"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

archive="sandbox-$target.tar.gz"
echo "Downloading $base/$archive"
fetch "$base/$archive" "$tmp/$archive" || fail "download failed: $base/$archive"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "download failed: $base/SHA256SUMS"

expected=$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
[ -n "$expected" ] || fail "$archive is not listed in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$archive" | awk '{ print $1 }')
else
    actual=$(shasum -a 256 "$tmp/$archive" | awk '{ print $1 }')
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch for $archive"

tar -xzf "$tmp/$archive" -C "$tmp" sandbox
mkdir -p "$install_dir"
# Write next to the target and rename, so a running sandbox isn't overwritten in place.
cp "$tmp/sandbox" "$install_dir/.sandbox.new"
chmod 755 "$install_dir/.sandbox.new"
mv -f "$install_dir/.sandbox.new" "$install_dir/sandbox"
if [ "$(uname -s)" = Darwin ]; then
    xattr -d com.apple.quarantine "$install_dir/sandbox" 2>/dev/null || true
fi

echo "Installed $("$install_dir/sandbox" --version) to $install_dir/sandbox"
case ":$PATH:" in
    *":$install_dir:"*) echo "Run: sandbox init  (in a project folder), then sandbox up. Update later with: sandbox upgrade" ;;
    *)
        echo
        echo "$install_dir is not on your PATH. Add this line to your shell profile (~/.profile, ~/.bashrc or ~/.zshrc):"
        echo "    export PATH=\"$install_dir:\$PATH\""
        echo "then open a new terminal and run: sandbox --version"
        ;;
esac
