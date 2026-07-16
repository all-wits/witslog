"""Regression check: assert the most recent event's `context` contains no `argv` key.

Used by run.ps1 after an "argv-off" smoke run (enrich.argv=false) to prove the
mitigation for the CLI-arg-secret exposure documented in CONTRACT.md actually
holds end-to-end, independent of which language SDK wrote the event.

Usage: py assert_no_argv.py <project_dir> <tag_token>
Exit 0 = argv absent (and pid still present, proving other enrichment untouched).
Exit 1 = regression: argv present, or the row wasn't found at all.
"""

import os
import sqlite3
import sys


def main():
    proj, tag = sys.argv[1], sys.argv[2]
    db = os.path.join(proj, ".witslog", "witslog.db")
    conn = sqlite3.connect(db)
    row = conn.execute(
        "SELECT context FROM events WHERE tags LIKE ? ORDER BY id DESC LIMIT 1",
        (f"%{tag}%",),
    ).fetchone()

    if row is None:
        print(f"FAIL: no event found with tag containing {tag!r}")
        return 1

    context = row[0] or ""
    if '"argv"' in context:
        print(f"FAIL: argv present in context despite enrich.argv=false: {context}")
        return 1

    if '"pid"' not in context:
        print(f"FAIL: pid missing too - enrich.argv=false must not disable other enrichment: {context}")
        return 1

    print("OK: argv absent, other enrichment (pid) intact")
    return 0


if __name__ == "__main__":
    sys.exit(main())
