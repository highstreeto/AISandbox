# shellcheck shell=bash
# Shared helpers for tools/*.sh (sourced, runs as root).
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
SHORTCUTS=/usr/local/share/sandbox/shortcuts
ANTHROPIC_FPR=31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE

apt_install() {
  apt-get update -q
  apt-get install -y -q --no-install-recommends "$@"
  rm -rf /var/lib/apt/lists/*
}

# add_apt_repo <name> <key-url> "<deb line; KEYRING is replaced by the key path>"
add_apt_repo() {
  local name=$1 key=$2 line=$3 kr=/etc/apt/keyrings/$1.asc
  install -d -m 0755 /etc/apt/keyrings
  curl -fsSL "$key" -o "$kr"
  echo "${line//KEYRING/$kr}" > "/etc/apt/sources.list.d/$name.list"
}

verify_anthropic_key() {
  gpg --show-keys --with-colons "$1" | grep -q "^fpr:.*$ANTHROPIC_FPR" \
    || { echo "Signing key $1 does not match Anthropic's fingerprint"; exit 1; }
}

# electron_wrapper <real binary> <wrapper path>
# Electron's own sandbox needs user namespaces, which Docker blocks; the
# container is the sandbox. The proxy is passed explicitly to Chromium.
electron_wrapper() {
  cat > "$2" <<EOF
#!/bin/sh
exec "$1" --no-sandbox --proxy-server="\${HTTPS_PROXY:-http://proxy:3128}" --proxy-bypass-list="localhost;127.0.0.1;::1" "\$@"
EOF
  chmod 0755 "$2"
}

# shortcut <id> <name> <exec> [icon]   (@WORKDIR@ = project folder, filled at start)
shortcut() {
  install -d "$SHORTCUTS"
  cat > "$SHORTCUTS/$1.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$2
Exec=$3
Icon=${4:-applications-development}
Terminal=false
Categories=Development;
EOF
}
