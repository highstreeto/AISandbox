#!/usr/bin/env bash
# ABOUT: OpenCode CLI v2 (opencode.ai/v2/install) into /opt/opencode. Option: version, e.g. 2.2.6
# HOSTS: opencode.ai github.com release-assets.githubusercontent.com (install) opencode.ai models.dev registry.npmjs.org (models, plugins) + your model provider
source "$(dirname "$0")/_lib.sh"
dest=/opt/opencode

# The installer writes to $HOME/.opencode/bin (or OPENCODE_INSTALL_DIR) and edits
# shell rc files; a throwaway HOME keeps that out of /root.
rm -rf "$dest" && mkdir -p "$dest"
curl -fsSL https://opencode.ai/v2/install \
  | env -u XDG_BIN_DIR HOME="$dest" OPENCODE_INSTALL_DIR="$dest/bin" bash -s -- ${1:+--version "$1"}
rm -f "$dest"/.bashrc "$dest"/.profile "$dest"/.zshrc "$dest"/.config/fish/config.fish

bin=""
for c in "$dest/bin/opencode" "$dest/.opencode/bin/opencode"; do
  if [ -x "$c" ]; then bin=$c; break; fi
done
[ -n "$bin" ] || { echo "opencode binary not found under $dest"; exit 1; }
chmod -R a+rX "$dest"

# NODE_USE_ENV_PROXY: Node's fetch ignores HTTPS_PROXY otherwise (Bun reads it anyway).
cat > /usr/local/bin/opencode <<WRAP
#!/bin/sh
export NODE_USE_ENV_PROXY=1
exec "$bin" "\$@"
WRAP
chmod 0755 /usr/local/bin/opencode
opencode --version
shortcut opencode "OpenCode" "konsole --workdir @WORKDIR@ -e bash -lc \"opencode; exec bash\"" utilities-terminal
