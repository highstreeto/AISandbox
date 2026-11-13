@echo off
rem AI Sandbox launcher (Windows cmd / PowerShell). Works from any directory; see README.md.
setlocal
set "SANDBOX_DIR=%~dp0.sandbox"
cargo run --quiet --locked --manifest-path "%~dp0Cargo.toml" --bin sandbox -- %*
