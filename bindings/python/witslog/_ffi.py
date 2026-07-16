"""ctypes binding to the native witslog library.

Pure stdlib — no third-party dependency. See ../CONTRACT.md for the ABI.
"""

import ctypes
import os
import platform
import sys
from pathlib import Path

from .errors import WitslogContractError, WitslogLibraryError

#: ABI contract version this SDK is built for. Keep in sync with
#: WITSLOG_ABI_VERSION in crates/witslog-ffi/src/lib.rs and bindings/CONTRACT.md.
ABI_VERSION = 1


def _platform_dir():
    """Return the bundled-lib subdir name, e.g. 'win32-x64', 'linux-x64', 'darwin-arm64'."""
    sysname = {"windows": "win32", "linux": "linux", "darwin": "darwin"}.get(
        platform.system().lower(), platform.system().lower()
    )
    machine = platform.machine().lower()
    arch = {
        "amd64": "x64",
        "x86_64": "x64",
        "aarch64": "arm64",
        "arm64": "arm64",
    }.get(machine, machine)
    return f"{sysname}-{arch}"


def _lib_filename():
    if sys.platform == "win32":
        return "witslog_ffi.dll"
    if sys.platform == "darwin":
        return "libwitslog_ffi.dylib"
    return "libwitslog_ffi.so"


def _candidate_paths():
    """Yield candidate paths in locator order (see CONTRACT.md §locator)."""
    paths = []
    env = os.environ.get("WITSLOG_LIB")
    if env:
        paths.append(env)
    bundled = Path(__file__).parent / "_libs" / _platform_dir() / _lib_filename()
    paths.append(str(bundled))
    # OS default loader: pass the bare filename to CDLL last.
    paths.append(_lib_filename())
    return paths


class _Lib:
    """Typed wrapper over the loaded CDLL. Validates the ABI version on construction."""

    def __init__(self, cdll, loaded_from):
        self._c = cdll
        self.loaded_from = loaded_from

        cdll.witslog_abi_version.argtypes = []
        cdll.witslog_abi_version.restype = ctypes.c_int32

        cdll.witslog_configure.argtypes = [ctypes.c_char_p]
        cdll.witslog_configure.restype = ctypes.c_int32

        cdll.witslog_init.argtypes = [ctypes.c_char_p]
        cdll.witslog_init.restype = ctypes.c_int32

        cdll.witslog_log.argtypes = [ctypes.c_char_p]
        cdll.witslog_log.restype = ctypes.c_int64

        cdll.witslog_resolve.argtypes = [ctypes.c_char_p]
        cdll.witslog_resolve.restype = ctypes.c_int32

        cdll.witslog_delete.argtypes = [ctypes.c_char_p]
        cdll.witslog_delete.restype = ctypes.c_void_p

        cdll.witslog_flush.argtypes = []
        cdll.witslog_flush.restype = ctypes.c_int32

        cdll.witslog_shutdown.argtypes = []
        cdll.witslog_shutdown.restype = ctypes.c_int32

        cdll.witslog_free_string.argtypes = [ctypes.c_void_p]
        cdll.witslog_free_string.restype = None

        actual = int(cdll.witslog_abi_version())
        if actual != ABI_VERSION:
            raise WitslogContractError(ABI_VERSION, actual)

    def abi_version(self):
        return int(self._c.witslog_abi_version())

    def configure(self, json_bytes):
        return int(self._c.witslog_configure(json_bytes))

    def init(self, json_bytes):
        return int(self._c.witslog_init(json_bytes))

    def log(self, json_bytes):
        return int(self._c.witslog_log(json_bytes))

    def resolve(self, event_id_bytes):
        return int(self._c.witslog_resolve(event_id_bytes))

    def delete(self, filter_json_bytes):
        ptr = self._c.witslog_delete(filter_json_bytes)
        if not ptr:
            return None
        try:
            return ctypes.cast(ptr, ctypes.c_char_p).value.decode("utf-8")
        finally:
            self._c.witslog_free_string(ptr)

    def flush(self):
        return int(self._c.witslog_flush())

    def shutdown(self):
        return int(self._c.witslog_shutdown())


def load_library():
    """Locate and load the native library, returning a validated `_Lib`.

    Raises `WitslogLibraryError` if no candidate loads, `WitslogContractError`
    on an ABI-version mismatch.
    """
    tried = _candidate_paths()
    last_os_error = None
    for candidate in tried:
        try:
            cdll = ctypes.CDLL(candidate)
        except OSError as exc:
            last_os_error = exc
            continue
        return _Lib(cdll, candidate)
    # Nothing loaded.
    raise WitslogLibraryError(tried) from last_os_error
