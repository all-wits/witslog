<div align="center">

# 🪵 witslog (Python SDK)

[![PyPI](https://img.shields.io/pypi/v/witslog?logo=pypi)](https://pypi.org/project/witslog/)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](../../LICENSE)
[![Python](https://img.shields.io/pypi/pyversions/witslog?logo=python)](https://pypi.org/project/witslog/)

**Framework-agnostic Python SDK for [witslog](../../README.md) structured error logging.**

</div>

---

Thin wrapper over the native `witslog-ffi` C ABI using pure **stdlib `ctypes`** — zero
third-party runtime dependencies. See [../CONTRACT.md](../CONTRACT.md) for the full
SDK↔native ABI.

## 📦 Install

```bash
pip install witslog
```

## 🚀 Quick Start

```python
import witslog

witslog.init()                       # mount once (installs atexit flush)
witslog.error("myapp", "db timeout", context={"request_id": "r1"}, tags=["db"])

try:
    risky()
except Exception:
    witslog.exception("myapp")        # captures the traceback
```

Run from a directory inside a `.witslog/` project (or `witslog init` one first) so events
land in that project's DB. This SDK doesn't yet wrap the native `witslog_bootstrap_project`
export (see [../CONTRACT.md](../CONTRACT.md#witslog_bootstrap_project-no-json--plain-path-string-or-null))
the way the [Node SDK's](../node) `init({ createProject: true })` does — you still need the
CLI installed separately for now.

> **🔒 Security:** `argv` enrichment defaults on and captures the full command line. If your
> app may receive secrets as bare CLI args, call
> `witslog.init({"enrich": {"argv": False}})` — see
> [../CONTRACT.md](../CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## 🧩 Web frameworks

```python
from witslog.frameworks.fastapi import add_witslog   # add_witslog(app)
from witslog.frameworks.flask import Witslog          # Witslog(app)
# Django: add "witslog.frameworks.django.WitslogMiddleware" to MIDDLEWARE
```

Install the framework extra to pull its dependency, e.g. `pip install witslog[fastapi]`.

## 📖 API

| Function | Description |
|----------|--------------|
| `init(config=None)` | Mount the SDK; optionally pass enrich/redact/buffer config. |
| `error/warn/info(app, message, **opts)` | Log at the given severity. `opts`: `context`, `tags`, `metadata`, `error_code`, `exception`, ... |
| `exception(app, **opts)` | Log the currently-handled exception, capturing its traceback. |
| `flush()` / `shutdown()` | Drain buffered events before exit. |

## 🌍 Platform support

No bundled native libraries yet — this package has no release CI matrix like
[the Node SDK's](../node) does. Point at a locally built `witslog-ffi` via:

```bash
WITSLOG_LIB=/path/to/witslog_ffi.{dll,so,dylib}
```

or drop the built lib under `witslog/_libs/<platform>/` (see
[../CONTRACT.md](../CONTRACT.md) for the platform-dir naming and locator order).
Cross-platform prebuilt bundling is tracked for a future release.

## 🧪 Test

```bash
py -m pytest tests
```

## 📄 License

Apache License 2.0 — see [../../LICENSE](../../LICENSE).
