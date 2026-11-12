@echo off
rem AI Sandbox launcher (Windows cmd / PowerShell). Works from any directory; see README.md.
setlocal
set "SANDBOX_DIR=%~dp0.sandbox"
dotnet run --project "%~dp0src\AiSandbox" --no-launch-profile -v q -- %*
