# Installs (or updates) the AI Sandbox app `sandbox` on Windows.
#
#   irm https://github.com/highstreeto/AISandbox/releases/latest/download/install.ps1 | iex
#
# $env:SANDBOX_VERSION = "v1.2.3"      install that release instead of the newest one
# $env:SANDBOX_INSTALL_DIR = "..."     install there instead of %LOCALAPPDATA%\Programs\sandbox
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repoUrl = 'https://github.com/highstreeto/AISandbox/releases'
$installDir = if ($env:SANDBOX_INSTALL_DIR) { $env:SANDBOX_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\sandbox' }

# Windows on ARM runs the x64 build through emulation.
$target = 'x86_64-pc-windows-msvc'

if ($env:SANDBOX_VERSION) {
    $tag = if ($env:SANDBOX_VERSION.StartsWith('v')) { $env:SANDBOX_VERSION } else { "v$($env:SANDBOX_VERSION)" }
    $base = "$repoUrl/download/$tag"
} else {
    $base = "$repoUrl/latest/download"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("sandbox-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $archive = "sandbox-$target.zip"
    Write-Host "Downloading $base/$archive"
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$archive" -OutFile (Join-Path $tmp $archive)
    Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')

    $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { ($_ -split '\s+')[1] -in @($archive, "*$archive") }
    if (-not $line) { throw "$archive is not listed in SHA256SUMS" }
    $expected = ($line -split '\s+')[0]
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $archive)).Hash
    if ($expected -ne $actual.ToLower()) { throw "checksum mismatch for $archive" }

    Expand-Archive -Path (Join-Path $tmp $archive) -DestinationPath (Join-Path $tmp 'pkg')
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    $exe = Join-Path $installDir 'sandbox.exe'
    # A running sandbox.exe can't be overwritten, but it can be renamed out of the way.
    if (Test-Path $exe) {
        $old = Join-Path $installDir 'sandbox.old.exe'
        Remove-Item $old -Force -ErrorAction SilentlyContinue
        Move-Item $exe $old -Force
    }
    Copy-Item (Join-Path $tmp 'pkg\sandbox.exe') $exe
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$entries = @($userPath -split ';' | Where-Object { $_ })
if ($entries -notcontains $installDir) {
    [Environment]::SetEnvironmentVariable('Path', (($entries + $installDir) -join ';'), 'User')
    Write-Host "Added $installDir to your user PATH (new terminals pick it up)."
}
if (($env:Path -split ';') -notcontains $installDir) {
    $env:Path = "$env:Path;$installDir"
}

Write-Host "Installed $(& $exe --version) to $exe"
Write-Host 'Run: sandbox init  (in a project folder), then sandbox up. Update later with: sandbox upgrade'
