---
name: verify
description: Project-specific runtime verification recipe for witslog (CLI, FFI, Node SDK CLI shim)
---

# witslog verify recipe

## Build

```powershell
cargo build --release -p witslog-ffi -p witslog-cli
```

Produces `target\release\witslog.exe` and `target\release\witslog_ffi.dll`.

## Drive the CLI directly

```powershell
$proj = Join-Path $env:TEMP ("wits_" + [guid]::NewGuid().ToString('N').Substring(0,8))
New-Item -ItemType Directory -Force -Path $proj | Out-Null
Push-Location $proj
& "C:\projects\witslog\target\release\witslog.exe" init .
& "C:\projects\witslog\target\release\witslog.exe" log app "hello" --severity error
& "C:\projects\witslog\target\release\witslog.exe" query "hello*"
Pop-Location
```

## Drive the Node SDK npm CLI shim (bin/witslog.js)

Not published yet — `_bin/<platform>/` is empty locally (only CI populates it on
publish). Point at the freshly built binary via env, same as a user with an
unbundled platform would:

```powershell
$env:WITSLOG_CLI = "C:\projects\witslog\target\release\witslog.exe"
$env:WITSLOG_LIB = "C:\projects\witslog\target\release\witslog_ffi.dll"
node C:\projects\witslog\bindings\node\bin\witslog.js init .
node C:\projects\witslog\bindings\node\bin\witslog.js query "*"
```

**Gotcha**: PowerShell 5.1 wraps native-command stderr in a terminating
`ErrorRecord` when piped through `2>&1` at the *outer* command level (e.g.
wrapping the whole `bindings\e2e\run.ps1` invocation) — cargo's normal
"Compiling ..." lines go to stderr and this makes the whole call look like it
failed even on exit code 0. Don't `2>&1` the outer `& .\bindings\e2e\run.ps1`
call; let it print directly.

## Full regression + e2e gate

```powershell
& C:\projects\witslog\bindings\e2e\run.ps1
```

Builds CLI+FFI, runs `cargo test --workspace`, then 9 gates: python/node/php
SDK smoke, argv-off mitigation lock (x3), browser ingest, npm CLI shim
(Gate 5 — `bin/witslog.js` -> real binary -> real DB).

## Node SDK unit/feature tests only

```bash
cd bindings/node && npm test
```

## Useful probes for the CLI shim specifically

- Unset `WITSLOG_CLI`, no `_bin/` populated → clean `WitslogCliNotFoundError`
  listing tried paths, not a raw ENOENT stack.
- Bogus subcommand → real clap exit code 2, usage printed, forwarded verbatim.
- `--help`/`--version` → forwarded, exit 0.
