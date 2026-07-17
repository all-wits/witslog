<#
    witslog SDK end-to-end + regression driver (Windows / PowerShell).

    Gate 1 - full workspace regression lock: `cargo test --workspace` runs every
             existing P0-P5 functional test (store/query/taxonomy/MCP/runtime/ffi)
             so new SDK work can never silently regress shipped behavior.
    Gate 2 - per-language SDK e2e: for Python/Node/PHP,
      1. inits a fresh temp .witslog project,
      2. runs the SDK smoke script (mount -> log w/ context+tags -> exception -> flush),
      3. asserts the CLI reads back BOTH the message marker AND the tags-only token
         (proving message + the `tags` ABI field crossed the FFI cross-process).
    Gate 3 - argv/secret-exposure mitigation lock: re-runs each smoke with
             enrich.argv=false and asserts (via direct DB read, language-agnostic)
             that `argv` is absent from the persisted context while other
             enrichment (pid) remains - proving the documented mitigation
             (CONTRACT.md "Security note") actually holds end-to-end.
    Gate 4 - browser ingest e2e (P10): posts a browser-shaped batch through
             `witslogBrowserIngest` (bindings/node/frameworks/express.js)
             backed by the REAL native FFI (not a fake lib), then reads the
             message back through the real CLI - proving client-side text
             actually crosses ingest -> FFI -> DB -> CLI, not just the
             mocked-lib unit tests.

    Usage:  pwsh bindings/e2e/run.ps1 [-SkipBuild] [-SkipWorkspaceTests]
    Exit code 0 = everything passed.
#>
param([switch]$SkipBuild, [switch]$SkipWorkspaceTests)

$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent  # repo root
$e2e = $PSScriptRoot
$dll = Join-Path $root 'target\release\witslog_ffi.dll'
$cli = Join-Path $root 'target\release\witslog.exe'

function Section($m) { Write-Host "`n=== $m ===" -ForegroundColor Cyan }

if (-not $SkipBuild) {
    Section 'cargo build --release -p witslog-ffi -p witslog-cli'
    Push-Location $root
    cargo build --release -p witslog-ffi -p witslog-cli
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    Pop-Location
}
if (-not (Test-Path $dll)) { throw "missing dll: $dll" }
if (-not (Test-Path $cli)) { throw "missing cli: $cli" }

$results = @{}

# --- Gate 1: full workspace regression lock ---------------------------------
if (-not $SkipWorkspaceTests) {
    Section 'cargo test --workspace (regression lock: all existing P0-P5 features)'
    Push-Location $root
    cargo test --workspace
    $workspaceOk = ($LASTEXITCODE -eq 0)
    Pop-Location
    $results['workspace-tests'] = $workspaceOk
    if (-not $workspaceOk) {
        Write-Host "workspace regression tests FAILED - stopping before SDK e2e" -ForegroundColor Red
    }
    else {
        Write-Host "workspace regression tests PASS" -ForegroundColor Green
    }
}
else {
    Write-Host 'skipping workspace regression tests (-SkipWorkspaceTests)' -ForegroundColor Yellow
}

function Invoke-Smoke {
    param(
        [string]$Name,
        [string]$Marker,
        [scriptblock]$Run   # receives $proj; runs the SDK smoke in-process
    )
    Section "$Name SDK e2e"
    $proj = Join-Path $env:TEMP ("wits_{0}_{1}" -f $Name, [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force -Path $proj | Out-Null
    $ok = $false
    try {
        Push-Location $proj
        & $cli init . | Out-Null
        $env:WITSLOG_LIB = $dll
        & $Run $proj
        if ($LASTEXITCODE -ne 0) { throw "$Name smoke exited $LASTEXITCODE" }

        $byMessage = & $cli query "$Marker*" 2>&1 | Out-String
        $byTag = & $cli query "TAG$Marker" 2>&1 | Out-String
        Pop-Location

        if ($byMessage -notmatch [regex]::Escape($Marker)) {
            throw "${Name}: CLI did not read back the message marker.`n$byMessage"
        }
        if ($byTag -notmatch [regex]::Escape($Marker)) {
            throw "${Name}: CLI did not read back via the tags-only token (tags field did not cross the ABI).`n$byTag"
        }
        Write-Host "$Name OK - message + tags round-tripped through the CLI" -ForegroundColor Green
        $ok = $true
    }
    catch {
        if ((Get-Location).Path -eq $proj) { Pop-Location }
        Write-Host "$Name FAILED: $_" -ForegroundColor Red
    }
    finally {
        Remove-Item -Recurse -Force $proj -ErrorAction SilentlyContinue
    }
    $results[$Name] = $ok
}

function Invoke-ArgvOffSmoke {
    param(
        [string]$Name,
        [string]$Marker,
        [scriptblock]$Run   # receives $proj; runs the SDK smoke in argv-off mode
    )
    Section "$Name SDK argv/secret-exposure mitigation lock"
    $proj = Join-Path $env:TEMP ("wits_{0}_argvoff_{1}" -f $Name, [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force -Path $proj | Out-Null
    $ok = $false
    try {
        Push-Location $proj
        & $cli init . | Out-Null
        $env:WITSLOG_LIB = $dll
        & $Run $proj
        if ($LASTEXITCODE -ne 0) { throw "$Name argv-off smoke exited $LASTEXITCODE" }
        Pop-Location

        $checkOut = py (Join-Path $e2e 'assert_no_argv.py') $proj $Marker 2>&1 | Out-String
        Write-Host $checkOut
        if ($checkOut -notmatch 'OK: argv absent') {
            throw "${Name}: enrich.argv=false did not suppress argv capture (regression!).`n$checkOut"
        }
        Write-Host "$Name OK - argv absent, mitigation holds end-to-end" -ForegroundColor Green
        $ok = $true
    }
    catch {
        if ((Get-Location).Path -eq $proj) { Pop-Location }
        Write-Host "$Name FAILED: $_" -ForegroundColor Red
    }
    finally {
        Remove-Item -Recurse -Force $proj -ErrorAction SilentlyContinue
    }
    $results["$Name-argv-off"] = $ok
}

# --- Gate 2: per-language SDK e2e (message + tags cross the ABI) -----------
$pyMarker = 'PY' + (Get-Random -Maximum 999999)
Invoke-Smoke -Name 'python' -Marker $pyMarker -Run {
    param($proj)
    $env:PYTHONPATH = Join-Path $root 'bindings\python'
    py (Join-Path $e2e 'py_smoke.py') $pyMarker 'argv-on'
}

$nodeMarker = 'ND' + (Get-Random -Maximum 999999)
Invoke-Smoke -Name 'node' -Marker $nodeMarker -Run {
    param($proj)
    $env:WITSLOG_PKG = Join-Path $root 'bindings\node\index.js'
    node (Join-Path $e2e 'node_smoke.js') $nodeMarker 'argv-on'
}

$phpMarker = 'PH' + (Get-Random -Maximum 999999)
Invoke-Smoke -Name 'php' -Marker $phpMarker -Run {
    param($proj)
    $env:WITSLOG_AUTOLOAD = Join-Path $root 'bindings\php\vendor\autoload.php'
    php -d extension=ffi -d ffi.enable=1 (Join-Path $e2e 'php_smoke.php') $phpMarker 'argv-on'
}

# --- Gate 3: argv/secret-exposure mitigation, per language ------------------
$pyArgvMarker = 'PYAO' + (Get-Random -Maximum 999999)
Invoke-ArgvOffSmoke -Name 'python' -Marker $pyArgvMarker -Run {
    param($proj)
    $env:PYTHONPATH = Join-Path $root 'bindings\python'
    py (Join-Path $e2e 'py_smoke.py') $pyArgvMarker 'argv-off'
}

$nodeArgvMarker = 'NDAO' + (Get-Random -Maximum 999999)
Invoke-ArgvOffSmoke -Name 'node' -Marker $nodeArgvMarker -Run {
    param($proj)
    $env:WITSLOG_PKG = Join-Path $root 'bindings\node\index.js'
    node (Join-Path $e2e 'node_smoke.js') $nodeArgvMarker 'argv-off'
}

$phpArgvMarker = 'PHAO' + (Get-Random -Maximum 999999)
Invoke-ArgvOffSmoke -Name 'php' -Marker $phpArgvMarker -Run {
    param($proj)
    $env:WITSLOG_AUTOLOAD = Join-Path $root 'bindings\php\vendor\autoload.php'
    php -d extension=ffi -d ffi.enable=1 (Join-Path $e2e 'php_smoke.php') $phpArgvMarker 'argv-off'
}

# --- Gate 4: browser ingest e2e (real FFI, not a fake lib) ------------------
Section 'browser ingest e2e'
$browserMarker = 'BR' + (Get-Random -Maximum 999999)
$proj = Join-Path $env:TEMP ("wits_browser_{0}" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force -Path $proj | Out-Null
$browserOk = $false
try {
    Push-Location $proj
    & $cli init . | Out-Null
    $env:WITSLOG_LIB = $dll
    $env:WITSLOG_PKG = Join-Path $root 'bindings\node\index.js'
    node (Join-Path $e2e 'browser_ingest_smoke.js') $browserMarker
    if ($LASTEXITCODE -ne 0) { throw "browser ingest smoke exited $LASTEXITCODE" }

    $readback = & $cli query "$browserMarker*" 2>&1 | Out-String
    Pop-Location

    if ($readback -notmatch [regex]::Escape($browserMarker)) {
        throw "browser ingest: CLI did not read back the posted marker.`n$readback"
    }
    Write-Host "browser ingest OK - POST body round-tripped through real FFI + CLI" -ForegroundColor Green
    $browserOk = $true
}
catch {
    if ((Get-Location).Path -eq $proj) { Pop-Location }
    Write-Host "browser ingest FAILED: $_" -ForegroundColor Red
}
finally {
    Remove-Item -Recurse -Force $proj -ErrorAction SilentlyContinue
}
$results['browser-ingest'] = $browserOk

Section 'summary'
$allOk = $true
$order = @('workspace-tests', 'python', 'node', 'php', 'python-argv-off', 'node-argv-off', 'php-argv-off', 'browser-ingest')
foreach ($k in $order) {
    if (-not $results.ContainsKey($k)) { continue }
    $status = if ($results[$k]) { 'PASS' } else { 'FAIL' }
    if (-not $results[$k]) { $allOk = $false }
    $color = if ($results[$k]) { 'Green' } else { 'Red' }
    Write-Host ("  {0,-18} {1}" -f $k, $status) -ForegroundColor $color
}
if ($allOk) { Write-Host "`nALL GATES PASSED" -ForegroundColor Green; exit 0 }
else { Write-Host "`nSOME GATES FAILED" -ForegroundColor Red; exit 1 }
