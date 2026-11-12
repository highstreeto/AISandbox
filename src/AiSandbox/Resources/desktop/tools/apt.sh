#!/usr/bin/env bash
# ABOUT: Additional Ubuntu packages. Option: package names
# HOSTS: archive.ubuntu.com security.ubuntu.com (ports.ubuntu.com on arm64)
source "$(dirname "$0")/_lib.sh"
[ $# -gt 0 ] || { echo "apt: no packages given"; exit 0; }
apt_install "$@"
