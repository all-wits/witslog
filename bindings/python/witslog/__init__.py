"""witslog — framework-agnostic Python SDK for structured error logging.

Thin wrapper over the native witslog library (see CONTRACT.md). Works in any
Python app; optional web-framework adapters live in `witslog.frameworks`.

    import witslog
    witslog.init()                       # mount once at startup
    witslog.error("myapp", "boom", context={"request_id": "r1"}, tags=["db"])
    try:
        ...
    except Exception:
        witslog.exception("myapp")        # captures traceback
    witslog.shutdown()                    # flush before exit (atexit does this too)
"""

import atexit
import json
import sys
import traceback

from .errors import (
    WitslogContractError,
    WitslogError,
    WitslogLibraryError,
    WitslogWriteError,
)

__all__ = [
    "init",
    "log",
    "error",
    "warn",
    "info",
    "exception",
    "flush",
    "shutdown",
    "install_excepthook",
    "Builder",
    "ABI_VERSION",
    "WitslogError",
    "WitslogLibraryError",
    "WitslogContractError",
    "WitslogWriteError",
]

from ._ffi import ABI_VERSION  # noqa: E402

_lib = None
_atexit_registered = False

_ALLOWED_FIELDS = {
    "severity",
    "version",
    "environment",
    "category",
    "error_code",
    "exception",
    "stacktrace",
    "correlation_id",
    "parent_event_id",
    "context",
    "tags",
    "metadata",
}


def _get_lib():
    global _lib
    if _lib is None:
        from ._ffi import load_library

        _lib = load_library()
    return _lib


def _build_payload(application, message, **fields):
    """Build the `witslog_log` JSON contract dict. Pure — unit-testable without the lib.

    Raises TypeError/ValueError on invalid input (FR-P6 error table).
    """
    if not isinstance(application, str):
        raise TypeError("application must be a str")
    if not isinstance(message, str):
        raise TypeError("message must be a str")

    payload = {"application": application, "message": message}
    for key, value in fields.items():
        if value is None:
            continue
        if key not in _ALLOWED_FIELDS:
            raise ValueError(f"unknown field: {key!r}")
        if key == "tags" and not isinstance(value, (list, tuple)):
            raise TypeError("tags must be a list of strings")
        payload[key] = list(value) if key == "tags" else value
    return payload


def _encode(payload):
    """Serialise a payload dict to NUL-safe UTF-8 bytes for the C ABI."""
    text = json.dumps(payload, ensure_ascii=False)
    try:
        return text.encode("utf-8")
    except UnicodeEncodeError as exc:  # pragma: no cover - str is always encodable
        raise ValueError(f"payload is not valid UTF-8: {exc}") from exc


def log(application, message, **fields):
    """Log an event. Returns the DB rowid (or 0 when buffering). Raises on FFI error."""
    payload = _build_payload(application, message, **fields)
    rc = _get_lib().log(_encode(payload))
    if rc < 0:
        raise WitslogWriteError(f"witslog_log failed (rc={rc}) for application={application!r}")
    return rc


def error(application, message, **fields):
    fields.setdefault("severity", "error")
    return log(application, message, **fields)


def warn(application, message, **fields):
    fields.setdefault("severity", "warn")
    return log(application, message, **fields)


def info(application, message, **fields):
    fields.setdefault("severity", "info")
    return log(application, message, **fields)


def exception(application, exc=None, message=None, **fields):
    """Log the current (or given) exception with its traceback captured.

    Call inside an `except` block, or pass an exception instance.
    """
    if exc is None:
        exc = sys.exc_info()[1]

    if exc is not None:
        fields.setdefault("exception", type(exc).__name__)
        tb = "".join(traceback.format_exception(type(exc), exc, exc.__traceback__))
        fields.setdefault("stacktrace", tb)
        if message is None:
            message = str(exc) or type(exc).__name__
    if message is None:
        message = "exception"

    fields.setdefault("severity", "error")
    return log(application, message, **fields)


def init(config=None):
    """Mount witslog for this process. `config` is the init/configure dict (see CONTRACT.md)."""
    global _atexit_registered
    payload = _encode(config) if config is not None else None
    rc = _get_lib().init(payload)
    if rc == -1:
        raise ValueError("witslog_init rejected the config JSON")
    if rc == -2:
        raise ValueError("witslog_init rejected an invalid redaction pattern")
    if not _atexit_registered:
        atexit.register(shutdown)
        _atexit_registered = True
    return rc


def flush():
    return _get_lib().flush()


def shutdown():
    return _get_lib().shutdown()


def install_excepthook(application="python"):
    """Route uncaught exceptions to witslog before the interpreter prints them."""
    previous = sys.excepthook

    def _hook(exc_type, exc_value, exc_tb):
        try:
            tb = "".join(traceback.format_exception(exc_type, exc_value, exc_tb))
            log(
                application,
                str(exc_value) or exc_type.__name__,
                severity="fatal",
                exception=exc_type.__name__,
                stacktrace=tb,
            )
            flush()
        except WitslogError:
            pass  # never let logging failure mask the original crash
        previous(exc_type, exc_value, exc_tb)

    sys.excepthook = _hook


class Builder:
    """Fluent builder mirroring the Rust EventBuilder. `.build()` logs and returns the rowid."""

    def __init__(self, application, message):
        self._application = application
        self._message = message
        self._fields = {}

    def _set(self, key, value):
        self._fields[key] = value
        return self

    def severity(self, s):
        return self._set("severity", s)

    def version(self, v):
        return self._set("version", v)

    def environment(self, e):
        return self._set("environment", e)

    def category(self, c):
        return self._set("category", c)

    def error_code(self, c):
        return self._set("error_code", c)

    def exception(self, e):
        return self._set("exception", e)

    def stacktrace(self, s):
        return self._set("stacktrace", s)

    def correlation_id(self, c):
        return self._set("correlation_id", c)

    def parent_event_id(self, p):
        return self._set("parent_event_id", p)

    def context(self, ctx):
        return self._set("context", ctx)

    def tags(self, tags):
        return self._set("tags", tags)

    def metadata(self, meta):
        return self._set("metadata", meta)

    def build(self):
        return log(self._application, self._message, **self._fields)
