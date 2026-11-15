#!/usr/bin/env bash
# ABOUT: OpenCode desktop app v2 (Electron, .deb from GitHub releases; beta on Linux). Option: version, e.g. 2.2.7
# HOSTS: api.github.com github.com release-assets.githubusercontent.com (install) opencode.ai models.dev registry.npmjs.org (models, plugins) + your model provider
source "$(dirname "$0")/_lib.sh"
case "$(uname -m)" in
  x86_64)  arch=amd64 ;;
  aarch64) arch=arm64 ;;
  *) echo "Unsupported architecture $(uname -m)"; exit 1 ;;
esac
repo=anomalyco/opencode
asset=opencode-desktop-linux-$arch.deb

# "latest" on GitHub is still the 1.x line: take the newest v2 release that has the .deb.
if [ -n "${1:-}" ]; then tag=v${1#v}
else
  tag=$(curl -fsSL "https://api.github.com/repos/$repo/releases?per_page=100" \
        | jq -r --arg a "$asset" 'map(select(.draft == false and .prerelease == false
              and (.tag_name | startswith("v2."))
              and any(.assets[]; .name == $a)))[0].tag_name // empty')
  [ -n "$tag" ] || { echo "No OpenCode v2 release with $asset found"; exit 1; }
fi
echo "OpenCode desktop $tag"

tmp=$(mktemp -d)
curl -fsSL "https://github.com/$repo/releases/download/$tag/$asset" -o "$tmp/$asset"
pkg=$(dpkg-deb -f "$tmp/$asset" Package)
apt-get update -q
apt-get install -y -q --no-install-recommends "$tmp/$asset"
rm -rf /var/lib/apt/lists/* "$tmp"

# The launcher comes from the package's .desktop file; /usr/bin/<name> is an
# update-alternatives link (not in dpkg -L), so resolve it to the real binary.
desktop_files=$(dpkg -L "$pkg" | grep -E '^/usr/share/applications/.*\.desktop$' || true)
[ -n "$desktop_files" ] || { echo "No .desktop file in package $pkg"; exit 1; }
# shellcheck disable=SC2086
exe=$(awk '/^Exec=/ { sub(/^Exec=/, ""); gsub(/"/, ""); split($0, a, " "); print a[1]; exit }' $desktop_files)
real=$(readlink -f "$(command -v "$exe" || echo "$exe")")
[ -x "$real" ] || { echo "Launcher '$exe' of $pkg not found"; exit 1; }

# Electron sandbox off (see _lib.sh), but no --proxy-server (unlike electron_wrapper):
# the app appends --proxy-bypass-list=<-loopback>, which overrides ours and would send
# its calls to the local service (127.0.0.1) through the proxy. Without --proxy-server
# Chromium takes the system settings (/etc/xdg/kioslaverc: HTTPS_PROXY / NO_PROXY),
# which bypass loopback. NODE_USE_ENV_PROXY: Node's fetch (main process, and the
# "opencode serve --service" it starts) ignores HTTPS_PROXY otherwise.
cat > /usr/local/bin/opencode-desktop <<WRAP
#!/bin/sh
export NODE_USE_ENV_PROXY=1
exec "$real" --no-sandbox "\$@"
WRAP
chmod 0755 /usr/local/bin/opencode-desktop

icon=""
for f in $desktop_files; do
  sed -i -E 's#^Exec=("[^"]*"|[^ ]+)#Exec=/usr/local/bin/opencode-desktop#' "$f"
  [ -n "$icon" ] || icon=$(sed -n 's/^Icon=//p' "$f" | head -1)
done
shortcut opencode-desktop "OpenCode Desktop" "/usr/local/bin/opencode-desktop %U" "${icon:-applications-development}"
