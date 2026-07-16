# FR-P7-005: measure memory footprint of a CLI one-shot command and an idle MCP
# server, per §Non-functional targets (CLI one-shot < 15 MB, idle MCP < 30 MB).
#
# Usage: pwsh scripts/measure_memory.ps1 [-Release]
param(
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$profileDir = if ($Release) { "release" } else { "debug" }
$buildFlag = if ($Release) { "--release" } else { "" }

Write-Output "Building witslog-cli ($profileDir)..."
if ($Release) {
    cargo build -p witslog-cli --release
} else {
    cargo build -p witslog-cli
}

$exe = Join-Path $root "target\$profileDir\witslog.exe"
if (-not (Test-Path $exe)) {
    throw "binary not found at $exe"
}

$tmpProject = Join-Path ([System.IO.Path]::GetTempPath()) ("witslog-memtest-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmpProject | Out-Null

try {
    Push-Location $tmpProject

    & $exe init . | Out-Null

    # --- CLI one-shot: peak working set of a single `witslog log` invocation ---
    # ArgumentList elements with embedded spaces need explicit quoting on Windows
    # PowerShell 5.1 - Start-Process does not auto-quote each array element.
    $proc = Start-Process -FilePath $exe -ArgumentList 'log', 'app', '"memory test message"' -PassThru -NoNewWindow
    $peakBytes = 0
    while (-not $proc.HasExited) {
        $proc.Refresh()
        if ($proc.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $proc.PeakWorkingSet64 }
        Start-Sleep -Milliseconds 5
    }
    $cliMb = [math]::Round($peakBytes / 1MB, 2)
    Write-Output "CLI one-shot ('witslog log') peak working set: $cliMb MB (target < 15 MB)"

    # --- Idle MCP server: RSS a few seconds after startup, before any request ---
    # `serve-mcp --stdio` reads line-delimited JSON-RPC until stdin EOF (see
    # server.rs::serve_stdio) - a redirected *file* as stdin hits EOF instantly
    # and the server exits before we can sample it. Use an open, unclosed pipe
    # (via System.Diagnostics.Process) instead, so the server genuinely blocks
    # waiting for input, matching how a real MCP client's long-lived pipe behaves.
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = "serve-mcp --stdio"
    $psi.WorkingDirectory = $tmpProject
    $psi.RedirectStandardInput = $true
    $psi.UseShellExecute = $false
    $mcpProc = [System.Diagnostics.Process]::Start($psi)
    Start-Sleep -Seconds 2
    $mcpProc.Refresh()
    if ($mcpProc.HasExited) {
        Write-Output "Idle 'serve-mcp' exited unexpectedly before sampling (exit code $($mcpProc.ExitCode)) - skipping"
    } else {
        $mcpMb = [math]::Round($mcpProc.WorkingSet64 / 1MB, 2)
        Write-Output "Idle 'serve-mcp' working set: $mcpMb MB (target < 30 MB)"
        Stop-Process -Id $mcpProc.Id -Force -ErrorAction SilentlyContinue
    }

    Pop-Location
} finally {
    Remove-Item -Recurse -Force $tmpProject -ErrorAction SilentlyContinue
}
