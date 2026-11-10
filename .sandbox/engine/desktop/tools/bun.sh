#!/usr/bin/env bash
# ABOUT: bun JavaScript runtime into /usr/local/bin. Option: version, e.g. 1.3.2
# HOSTS: bun.sh github.com objects.githubusercontent.com (install) registry.npmjs.org (packages)
source "$(dirname "$0")/_lib.sh"

command -v unzip >/dev/null || apt_install unzip
curl -fsSL https://bun.sh/install | BUN_INSTALL=/usr/local bash -s ${1:+"bun-v$1"}
rm -rf /root/.bun
bun --version
