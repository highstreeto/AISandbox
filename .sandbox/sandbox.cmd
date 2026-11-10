@echo off
rem Launcher (Windows cmd / PowerShell): compiles (cached) and runs sandbox.cs with the .NET 10+ SDK.
rem `run --file ... --` hands every argument to the app (plain `dotnet file.cs` would keep --help, -v, ... for itself).
dotnet run --file "%~dp0sandbox.cs" -- %*
