"""Python SDK e2e smoke: mount, log (with context+tags), flush.

Run from inside a `.witslog/` project dir with WITSLOG_LIB pointing at the built
library and PYTHONPATH including bindings/python. Prints the marker + event_id-free
confirmation; the CLI readback is asserted by the driver (run.ps1).

Usage: py py_smoke.py <marker>
"""

import sys

import witslog


def main():
    marker = sys.argv[1] if len(sys.argv) > 1 else "PYSMOKE"
    argv_mode = sys.argv[2] if len(sys.argv) > 2 else "argv-on"
    print("abi", witslog.ABI_VERSION)

    if argv_mode == "argv-off":
        # Regression lock: enrich.argv=false must fully suppress argv capture,
        # closing the CLI-arg-secret exposure documented in CONTRACT.md.
        witslog.init({"enrich": {"argv": False}})
    else:
        witslog.init()

    rowid = witslog.error(
        "py-e2e",
        f"python sdk event {marker}",
        context={"request_id": f"{marker}-req", "pid": 1},
        tags=[marker, "python", f"TAG{marker}"],
        metadata={"lang": "python"},
    )
    print("rowid", rowid)
    assert rowid >= 0, "log returned an error"

    # exception path stores a stacktrace
    try:
        raise RuntimeError(f"boom {marker}")
    except RuntimeError:
        witslog.exception("py-e2e", tags=[marker])

    witslog.shutdown()
    print("PY_SMOKE_OK", marker)


if __name__ == "__main__":
    main()
