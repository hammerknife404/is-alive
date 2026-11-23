#!/usr/bin/env python3
"""Checks on the bundled skills' scripts that is-alive relies on.

    python3 scripts/test_skill_scripts.py

Kept here, not in skills/, so the bundled copies stay as the owner's
source has them. CI runs it with the validator's own suite.
"""

import gzip
import io
import json
import sys
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "skills/unifidump/scripts"))
sys.path.insert(0, str(ROOT / "skills/mapnetwork/scripts"))
import extract  # noqa: E402
import validate_map as vm  # noqa: E402

failed = 0


def check(name, ok, detail=""):
    global failed
    print(("PASS  " if ok else "FAIL  ") + name + ("" if ok else f": {detail}"))
    failed += not ok


def refused(path):
    """The message extract.py stops with, or None if it read the file."""
    try:
        extract.load_export_documents(path)
    except ValueError as e:
        return str(e)
    return None


with tempfile.TemporaryDirectory() as d:
    d = Path(d)
    # An encrypted backup: random-looking bytes, a multiple of 16 long.
    enc = bytes((i * 151 + 7) % 256 for i in range(4096))
    for name in ("autobackup.unf", "console.unifi", "backup.bin"):
        (d / name).write_bytes(enc)
        msg = refused(str(d / name))
        check(f"{name} is refused, not misread", msg is not None and "Config Export" in msg,
              repr(msg))
    # A zip holding an encrypted .unf says the same, not "no db.gz".
    z = io.BytesIO()
    with zipfile.ZipFile(z, "w") as zf:
        zf.writestr("network_site_x_unf/autobackup.unf", enc)
    (d / "wrapped_unf.zip").write_bytes(z.getvalue())
    msg = refused(str(d / "wrapped_unf.zip"))
    check("a zip of an encrypted .unf says it's encrypted",
          msg is not None and "encrypted" in msg, repr(msg))
    # What still reads: JSON (with a BOM and leading space), and a bare
    # db.gz.
    docs = [{"__cmd": "select", "collection": "site"}, {"name": "default"}]
    (d / "dump.json").write_bytes(b"\xef\xbb\xbf  " + json.dumps(docs).encode())
    check("JSON with a BOM and leading space reads",
          extract.load_export_documents(str(d / "dump.json")) == docs)
    (d / "plain.json").write_text(json.dumps(docs))
    check("plain JSON reads", extract.load_export_documents(str(d / "plain.json")) == docs)
    (d / "db.gz").write_bytes(gzip.compress(b""))
    check("a bare db.gz reads", extract.load_export_documents(str(d / "db.gz")) == [])

# The validator stops anything in SITE DATA that would end the page's
# <script> element, whatever case and wherever it is.
template = (ROOT / "skills/mapnetwork/assets/Site Map Template.dc.html").read_text(encoding="utf-8")
check("the template passes", vm.raw_html_errors(template) == [])
for bad in ("</script><img src=x onerror=alert(1)>", "</SCRIPT >", "<!-- x", "<Script>"):
    page = template.replace("this.NODES = [", f"this.NODES = [ // {bad}\n", 1)
    check(f"{bad!r} in SITE DATA is an error", len(vm.raw_html_errors(page)) == 1)
escaped = template.replace("this.NODES = [", "this.NODES = [ // \\u003c/script>\n", 1)
check("an escaped < passes", vm.raw_html_errors(escaped) == [])

print(f"\n{'all passed' if not failed else f'{failed} failed'}")
sys.exit(1 if failed else 0)
