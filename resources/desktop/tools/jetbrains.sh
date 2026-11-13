#!/usr/bin/env bash
# ABOUT: JetBrains tools (latest) into /opt/jetbrains. Option: toolbox rider idea pycharm webstorm clion goland rustrover datagrip phpstorm rubymine
# HOSTS: data.services.jetbrains.com download.jetbrains.com download-cdn.jetbrains.com (install) plugins.jetbrains.com account.jetbrains.com (plugins, licensing)
source "$(dirname "$0")/_lib.sh"
[ $# -gt 0 ] || set -- toolbox

declare -A CODE=( [toolbox]=TBA [rider]=RD [idea]=IIU [pycharm]=PCP [webstorm]=WS
                  [clion]=CL [goland]=GO [rustrover]=RR [datagrip]=DG
                  [phpstorm]=PS [rubymine]=RM )
declare -A TITLE=( [toolbox]="JetBrains Toolbox" [rider]="Rider" [idea]="IntelliJ IDEA"
                   [pycharm]="PyCharm" [webstorm]="WebStorm" [clion]="CLion"
                   [goland]="GoLand" [rustrover]="RustRover" [datagrip]="DataGrip"
                   [phpstorm]="PhpStorm" [rubymine]="RubyMine" )
case "$(uname -m)" in
  x86_64)  platform=linux ;;
  aarch64) platform=linuxARM64 ;;
  *) echo "Unsupported architecture $(uname -m)"; exit 1 ;;
esac

apt_install libxtst6 libxi6 libxrender1 libnss3

for name in "$@"; do
  code="${CODE[$name]:-}"
  [ -n "$code" ] || { echo "Unknown JetBrains tool '$name'"; exit 1; }
  url=$(curl -fsSL "https://data.services.jetbrains.com/products/releases?code=${code}&latest=true&type=release" \
        | jq -r --arg c "$code" --arg p "$platform" '.[$c][0].downloads[$p].link')
  [ -n "$url" ] && [ "$url" != "null" ] || { echo "No download for $name/$platform"; exit 1; }

  dest=/opt/jetbrains/$name
  rm -rf "$dest" && mkdir -p "$dest"
  curl -fsSL "$url" | tar -xz --strip-components=1 -C "$dest"

  launcher=""
  for c in "$dest/bin/$name" "$dest/bin/$name.sh" "$dest/bin/jetbrains-toolbox" "$dest/jetbrains-toolbox"; do
    if [ -x "$c" ]; then launcher=$c; break; fi
  done
  [ -n "$launcher" ] || launcher=$(find "$dest/bin" -maxdepth 1 -name '*.sh' -perm -u+x | head -1)
  [ -n "$launcher" ] || { echo "Launcher not found in $dest"; exit 1; }
  ln -sf "$launcher" "/usr/local/bin/jetbrains-$name"

  icon=$(find "$dest" -maxdepth 3 \( -name '*.svg' -o -name '*.png' \) | grep -iE "/(${name}|toolbox|jetbrains)[^/]*\.(svg|png)$" | head -1 || true)
  if [ "$name" = toolbox ]; then exec_line="/usr/local/bin/jetbrains-$name"
  else exec_line="/usr/local/bin/jetbrains-$name @WORKDIR@"; fi
  shortcut "jetbrains-$name" "${TITLE[$name]}" "$exec_line" "$icon"
done
