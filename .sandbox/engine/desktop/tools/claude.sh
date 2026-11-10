#!/usr/bin/env bash
# ABOUT: Claude Desktop app (Linux beta, Anthropic apt repo). Option: stable
# HOSTS: downloads.claude.ai (install only)
source "$(dirname "$0")/_lib.sh"
channel=${1:-stable}

add_apt_repo claude-desktop https://downloads.claude.ai/claude-desktop/key.asc \
  "deb [arch=amd64,arm64 signed-by=KEYRING] https://downloads.claude.ai/claude-desktop/apt/$channel $channel main"
verify_anthropic_key /etc/apt/keyrings/claude-desktop.asc
# Keep the repo file the package would add from duplicating ours.
echo 'CLAUDE_DESKTOP_ADD_REPO="false"' > /etc/default/claude-desktop
# --no-install-recommends: skips QEMU (Cowork VMs need KVM, not available in a
# container) and gnome-keyring (conflicts with KDE Wallet).
apt_install claude-desktop

real=$(dpkg -L claude-desktop | grep -E '/bin/claude-desktop$' | head -1)
electron_wrapper "${real:-/usr/bin/claude-desktop}" /usr/local/bin/claude-desktop
icon=""
for f in /usr/share/applications/*.desktop; do
  grep -q 'claude-desktop' "$f" 2>/dev/null || continue
  sed -i -E 's#^Exec=(/usr/bin/)?claude-desktop#Exec=/usr/local/bin/claude-desktop#' "$f"
  [ -n "$icon" ] || icon=$(sed -n 's/^Icon=//p' "$f" | head -1)
done
shortcut claude "Claude" "/usr/local/bin/claude-desktop %U" "${icon:-applications-internet}"
