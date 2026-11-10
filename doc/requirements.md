---
lang: en
---

# AI Sandbox – Requirements

An isolated environment for working on a repository or project with AI.

## Requirements

- Should be completely isolated from the host system → no network access / file access (as far as possible)
- Should include all development tools (e.g. Claude Code, VS Code, Godot, ...)
  - Ideally as GUI tools
- Should be built on container technologies and run on Windows and Linux
- Should also support working with local LLMs
- Should make it easy to share files with the sandbox (and revoke access again)
  - Modes: read-only and read-write
- Should be as easy to use as installing tools locally
