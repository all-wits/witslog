"""FFI-boundary error handling — uses fakes, no native library required."""

import os

import pytest

import witslog
from witslog import _ffi
from witslog.errors import (
    WitslogContractError,
    WitslogLibraryError,
    WitslogWriteError,
)


class _FakeFn:
    """Callable that ignores ctypes argtypes/restype assignment."""

    def __init__(self, ret):
        self._ret = ret
        self.argtypes = None
        self.restype = None

    def __call__(self, *args):
        return self._ret


class _FakeCDLL:
    def __init__(self, abi=_ffi.ABI_VERSION, log_ret=1):
        self.witslog_abi_version = _FakeFn(abi)
        self.witslog_configure = _FakeFn(0)
        self.witslog_init = _FakeFn(0)
        self.witslog_log = _FakeFn(log_ret)
        self.witslog_resolve = _FakeFn(0)
        self.witslog_delete = _FakeFn(0)
        self.witslog_flush = _FakeFn(0)
        self.witslog_shutdown = _FakeFn(0)
        self.witslog_free_string = _FakeFn(None)


def test_missing_library_raises_library_error(monkeypatch):
    bogus = os.path.join(os.sep, "no", "such", "witslog_ffi_xyz.dll")
    monkeypatch.setenv("WITSLOG_LIB", bogus)
    # Force the OS-default candidate to also miss.
    monkeypatch.setattr(_ffi, "_lib_filename", lambda: "witslog_ffi_definitely_absent.dll")
    with pytest.raises(WitslogLibraryError) as ei:
        _ffi.load_library()
    assert bogus in ei.value.searched_paths


def test_contract_mismatch_raises(monkeypatch):
    with pytest.raises(WitslogContractError) as ei:
        _ffi._Lib(_FakeCDLL(abi=999), "fake")
    assert ei.value.expected == _ffi.ABI_VERSION
    assert ei.value.actual == 999


def test_matching_abi_constructs_lib():
    lib = _ffi._Lib(_FakeCDLL(abi=_ffi.ABI_VERSION), "fake")
    assert lib.abi_version() == _ffi.ABI_VERSION


def test_write_error_on_negative_return(monkeypatch):
    fake = _ffi._Lib(_FakeCDLL(log_ret=-1), "fake")
    monkeypatch.setattr(witslog, "_lib", fake)
    with pytest.raises(WitslogWriteError):
        witslog.log("app", "boom")


def test_log_returns_rowid(monkeypatch):
    fake = _ffi._Lib(_FakeCDLL(log_ret=42), "fake")
    monkeypatch.setattr(witslog, "_lib", fake)
    assert witslog.log("app", "ok", context={"a": 1}) == 42


def test_init_forwards_argv_disable_config(monkeypatch):
    # Regression lock: an app that may pass secrets as bare CLI args must be able
    # to fully close that exposure by disabling argv enrichment. The suppression
    # itself is proven natively (witslog-ffi::configure_argv_false_suppresses_argv_capture);
    # this locks that the Python `init()` surface forwards the config unchanged.
    captured = {}
    fake = _ffi._Lib(_FakeCDLL(abi=_ffi.ABI_VERSION), "fake")

    def _spy_init(json_bytes):
        captured["payload"] = json_bytes
        return 0

    monkeypatch.setattr(fake, "init", _spy_init)
    monkeypatch.setattr(witslog, "_lib", fake)
    monkeypatch.setattr(witslog, "_atexit_registered", True)  # skip atexit registration noise

    witslog.init({"enrich": {"argv": False}})

    payload = captured["payload"].decode("utf-8")
    assert '"argv":false' in payload or '"argv": false' in payload


def test_exception_captures_traceback(monkeypatch):
    captured = {}
    fake = _ffi._Lib(_FakeCDLL(log_ret=1), "fake")
    monkeypatch.setattr(witslog, "_lib", fake)
    def _spy(b):
        captured["payload"] = b
        return 1

    monkeypatch.setattr(fake, "log", _spy)

    try:
        raise ValueError("kaboom")
    except ValueError:
        witslog.exception("app")

    payload = captured["payload"].decode("utf-8")
    assert "ValueError" in payload
    assert "kaboom" in payload
    assert "stacktrace" in payload
