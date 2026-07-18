# Installing witslog

## Quick install

**Linux / macOS**

```sh
curl -fsSL https://raw.githubusercontent.com/all-wits/witslog/main/install/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://raw.githubusercontent.com/all-wits/witslog/main/install/install.ps1 | iex
```

Both scripts detect OS/arch, download the matching release asset from GitHub
Releases, verify its SHA-256 checksum, and place `witslog` on PATH
(`~/.local/bin` on Linux/macOS, `%LOCALAPPDATA%\witslog\bin` on Windows) —
**automatically**, not just as a printed suggestion: Windows updates the User
`Path` env var (persists across terminals) plus the current session; Linux/
macOS appends an `export PATH=...` line to the detected shell rc file
(`.zshrc`/`.bashrc`/`.profile`, whichever `$SHELL` points at) plus the current
session, so the version check at the end of the script always succeeds. New
terminals pick it up automatically; re-running the installer is idempotent
(won't duplicate the PATH entry). Override with `WITSLOG_VERSION` /
`WITSLOG_INSTALL_DIR` env vars.

## Package managers

- **cargo**: `cargo install witslog-cli` (builds from source; works on any
  platform Rust supports, including ones without a prebuilt binary).
- **Homebrew** (planned tap): `brew install witslog/tap/witslog`.
- **Scoop** (planned bucket): `scoop bucket add witslog https://github.com/all-wits/scoop-bucket && scoop install witslog`.

`cargo install` plus the npm/pip/composer SDK packages already give
cross-platform distribution independent of a cut binary release; a winget
manifest and `.deb`/`.rpm` packages are deliberately not maintained yet —
there's no cut release to package against pre-1.0.

Package-manager manifests are generated from the same release artifacts as
the install scripts (see `.github/workflows/release.yml`).

## Verifying the install

```sh
witslog --version
witslog doctor
```

`witslog doctor` prints the binary version, the maximum schema version it
supports, and the resolved project DB — useful for confirming PATH setup and
diagnosing version-compat issues after an upgrade.

## First project

```sh
cd my-project
witslog init .
witslog log myapp "hello witslog"
witslog query "hello"
```

## MCP client registration

```sh
witslog serve-mcp --print-mcp-config
```

prints a generic `mcpServers` JSON snippet (command/args/cwd) that any MCP
client can drop into its config to launch `witslog serve-mcp --stdio` against
the current project.

## Upgrading

Re-run the install script (or your package manager's upgrade command). The
binary applies pending schema migrations lazily on first use, always
snapshotting the DB to a `.bak` file first (`witslog migrate` does this
explicitly; other commands do it implicitly via the same code path). If a
migration fails, the pre-migration `.bak` is restored automatically and the
command reports the failure — the live DB is never left half-migrated.

If your project DB carries a schema version newer than what the installed
binary supports (e.g. you rolled back to an older binary), every command
refuses with an "upgrade witslog" message rather than risking data loss.

## Uninstalling

```sh
witslog uninstall           # removes the binary only
witslog uninstall --purge   # also removes the current project's .witslog/
                             # directory and the global config directory
```

On Windows, a running executable cannot delete itself; `uninstall` prints the
exact `del` command to run after the process exits. On Linux/macOS the binary
unlinks itself immediately.

Package-manager installs should instead use the package manager's own
`uninstall`/`remove` command.
