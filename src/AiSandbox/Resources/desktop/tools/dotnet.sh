#!/usr/bin/env bash
# ABOUT: .NET SDK(s) into /usr/share/dotnet. Option: channel(s), e.g. 10.0 8.0 (default 10.0)
# HOSTS: dot.net builds.dotnet.microsoft.com (+ api.nuget.org for restores)
source "$(dirname "$0")/_lib.sh"
[ $# -gt 0 ] || set -- 10.0

# Native dependency: ICU (package name differs between Ubuntu releases).
apt-get update -q
icu=$(apt-cache search --names-only '^libicu[0-9]+$' | awk '{print $1}' | sort -V | tail -1)
apt_install "$icu" zlib1g

curl -fsSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh
for ch in "$@"; do
  bash /tmp/dotnet-install.sh --channel "$ch" --install-dir /usr/share/dotnet --no-path
done
rm -f /tmp/dotnet-install.sh
ln -sf /usr/share/dotnet/dotnet /usr/local/bin/dotnet
dotnet --list-sdks
