# witslog (Python SDK)

Framework-agnostic Python SDK over the native witslog library. No third-party runtime deps
(pure `ctypes`). See [../CONTRACT.md](../CONTRACT.md) for the ABI.

```python
import witslog

witslog.init()                       # mount once (installs atexit flush)
witslog.error("myapp", "db timeout", context={"request_id": "r1"}, tags=["db"])

try:
    risky()
except Exception:
    witslog.exception("myapp")        # captures the traceback
```

Locate the native library via the `WITSLOG_LIB` env var (dev/CI), or bundle it under
`witslog/_libs/<platform>/`. Run from a directory inside a `.witslog/` project (or `witslog init`
one first) so events land in that project's DB.

> **Security:** `argv` enrichment defaults on and captures the full command line. If your app
> may receive secrets as bare CLI args, call `witslog.init({"enrich": {"argv": False}})` — see
> [../CONTRACT.md](../CONTRACT.md#security-note-argv-enrichment-vs-secrets).

## Web frameworks

```python
from witslog.frameworks.fastapi import add_witslog   # add_witslog(app)
from witslog.frameworks.flask import Witslog          # Witslog(app)
# Django: add "witslog.frameworks.django.WitslogMiddleware" to MIDDLEWARE
```

## Test

```
py -m pytest tests
```
