#!/usr/bin/env bash
# ABOUT: Node.js + npm (NodeSource apt repo). Option: major version (default 24)
# HOSTS: deb.nodesource.com (install) registry.npmjs.org (npm install)
source "$(dirname "$0")/_lib.sh"
major=${1:-24}

add_apt_repo nodesource https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
  "deb [signed-by=KEYRING] https://deb.nodesource.com/node_$major.x nodistro main"
apt_install nodejs
node --version
