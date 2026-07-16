"""Exception types raised by the witslog SDK at the FFI boundary."""


class WitslogError(Exception):
    """Base class for all witslog SDK errors."""


class WitslogLibraryError(WitslogError):
    """The native witslog library could not be located or loaded."""

    def __init__(self, searched_paths):
        self.searched_paths = list(searched_paths)
        joined = "\n  ".join(self.searched_paths) or "(none)"
        super().__init__(
            "could not locate the native witslog library. Set the WITSLOG_LIB "
            "environment variable to its path, or bundle it under _libs/<platform>/. "
            f"Searched:\n  {joined}"
        )


class WitslogContractError(WitslogError):
    """The native library speaks a different ABI/contract version than this SDK."""

    def __init__(self, expected, actual):
        self.expected = expected
        self.actual = actual
        super().__init__(
            f"witslog contract mismatch: SDK expects ABI version {expected}, "
            f"native library reports {actual}. Upgrade the SDK or the native library."
        )


class WitslogWriteError(WitslogError):
    """The native library rejected a log/resolve call (returned -1)."""
