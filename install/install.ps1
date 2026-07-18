# witslog installer for Windows (FR-P8-002).
# Detects arch, downloads the matching release asset from GitHub Releases,
# verifies its checksum, and places the binary on PATH.
#
# Usage: irm https://.../install.ps1 | iex
# Env overrides: $env:WITSLOG_VERSION (default: latest), $env:WITSLOG_INSTALL_DIR (default: %LOCALAPPDATA%\witslog\bin)

$ErrorActionPreference = "Stop"

$Repo = "all-wits/witslog"
$Version = if ($env:WITSLOG_VERSION) { $env:WITSLOG_VERSION } else { "latest" }
$InstallDir = if ($env:WITSLOG_INSTALL_DIR) { $env:WITSLOG_INSTALL_DIR } else { "$env:LOCALAPPDATA\witslog\bin" }

$ArchRaw = $env:PROCESSOR_ARCHITECTURE
$Arch = switch ($ArchRaw) {
  "AMD64" { "x86_64" }
  # Windows on ARM has no prebuilt release asset yet (release.yml only builds
  # x86_64-pc-windows-msvc) - fall through to "unsupported" like any other
  # arch we don't ship, rather than attempting a download that 404s.
  default { "unsupported" }
}

if ($Arch -eq "unsupported") {
  Write-Error "no prebuilt witslog binary for this arch ($ArchRaw). Install via cargo instead: cargo install witslog-cli"
  exit 1
}

$Asset = "witslog-windows-$Arch.zip"

if ($Version -eq "latest") {
  $Url = "https://github.com/$Repo/releases/latest/download/$Asset"
  $ChecksumUrl = "https://github.com/$Repo/releases/latest/download/$Asset.sha256"
} else {
  $Url = "https://github.com/$Repo/releases/download/$Version/$Asset"
  $ChecksumUrl = "https://github.com/$Repo/releases/download/$Version/$Asset.sha256"
}

$TmpDir = Join-Path $env:TEMP ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
  Write-Host "Downloading $Asset ($Version)..."
  Invoke-WebRequest -Uri $Url -OutFile (Join-Path $TmpDir $Asset)
  Invoke-WebRequest -Uri $ChecksumUrl -OutFile (Join-Path $TmpDir "$Asset.sha256")

  Write-Host "Verifying checksum..."
  $expected = (Get-Content (Join-Path $TmpDir "$Asset.sha256")).Split(" ")[0].Trim()
  $actual = (Get-FileHash (Join-Path $TmpDir $Asset) -Algorithm SHA256).Hash.ToLower()
  if ($expected.ToLower() -ne $actual) {
    Write-Error "checksum verification failed, aborting install."
    exit 1
  }

  Expand-Archive -Path (Join-Path $TmpDir $Asset) -DestinationPath $TmpDir -Force

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Copy-Item (Join-Path $TmpDir "witslog.exe") (Join-Path $InstallDir "witslog.exe") -Force

  Write-Host "✓ witslog installed to $InstallDir\witslog.exe"

  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  if ($userPath -notlike "*$InstallDir*") {
    $newUserPath = if ([string]::IsNullOrEmpty($userPath)) { $InstallDir } else { "$userPath;$InstallDir" }
    [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
    # Update the current session too, so `witslog` works immediately without
    # reopening the terminal (the User env var change above only affects new processes).
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "✓ added $InstallDir to your User PATH (new terminals pick it up automatically)"
  }

  & (Join-Path $InstallDir "witslog.exe") --version
}
finally {
  Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}
