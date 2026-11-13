#!/usr/bin/env bash
# ABOUT: Claude Code CLI (Anthropic apt repo). Option: stable | latest
# HOSTS: downloads.claude.ai (install only)
source "$(dirname "$0")/_lib.sh"
channel=${1:-stable}

add_apt_repo claude-code https://downloads.claude.ai/keys/claude-code.asc \
  "deb [signed-by=KEYRING] https://downloads.claude.ai/claude-code/apt/$channel $channel main"
verify_anthropic_key /etc/apt/keyrings/claude-code.asc
apt_install claude-code
claude --version
shortcut claude-code "Claude Code" "konsole --workdir @WORKDIR@ -e bash -lc \"claude; exec bash\"" utilities-terminal
