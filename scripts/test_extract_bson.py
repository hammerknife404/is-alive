#!/usr/bin/env python3
"""The bundled /unifidump extract.py against damaged and crafted BSON.

    python3 scripts/test_extract_bson.py

Each case must end with a ValueError (or a result) quickly: no endless
loop, no spinning, no unbounded memory. Kept out of skills/ so the bundled
copy stays as the owner's source has it.
"""

import gzip
import io
import struct
import sys
import time
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "skills/unifidump/scripts"))
import extract  # noqa: E402

failed = 0


def check(name, ok, detail=""):
    global failed
    print(("PASS  " if ok else "FAIL  ") + name + ("" if ok else f": {detail}"))
    failed += not ok


def doc(body: bytes) -> bytes:
    return struct.pack("<i", len(body) + 5) + body + b"\x00"


def outcome(data):
    """('ok', docs) or ('err', message), and how long it took."""
    t = time.monotonic()
    try:
        r = ("ok", extract.decode_bson_stream(data))
    except ValueError as e:
        r = ("err", str(e))
    return r, time.monotonic() - t


good = doc(b"\x02name\x00" + struct.pack("<i", 4) + b"sw1\x00")
(r, _) = outcome(good)
check("a good document reads", r == ("ok", [{"name": "sw1"}]), r)

cases = {
    # Zero padding after the last document: the end, not an endless loop.
    "zero padding": good + b"\x00" * 64,
    # A string whose length points backwards: spun at 100% CPU before.
    "negative string length": doc(b"\x02k\x00" + struct.pack("<i", -8) + b"abcd"),
    "string past the end": doc(b"\x02k\x00" + struct.pack("<i", 1000) + b"ab\x00"),
    "document length past the stream": struct.pack("<i", 1 << 20) + b"\x00" * 16,
    "embedded document of length 0": doc(b"\x03k\x00" + struct.pack("<i", 0)),
    "truncated double": doc(b"\x01k\x00\x01\x02"),
    "code with scope pointing back": doc(b"\x0fk\x00" + struct.pack("<i", -100)),
}
for name, data in cases.items():
    (r, took) = outcome(data)
    ok = took < 2 and (r[0] == "err" or (name == "zero padding" and r == ("ok", [{"name": "sw1"}])))
    check(name, ok, f"{r} in {took:.2f}s")

# Nesting past MAX_DEPTH: refused, not a RecursionError.
inner = doc(b"")
for _ in range(extract.MAX_DEPTH + 5):
    inner = doc(b"\x03k\x00" + inner)
(r, took) = outcome(inner)
check("nesting past the limit", r[0] == "err" and "nested" in r[1], r)

# A gzip bomb is cut off at the cap, not unpacked whole. (The cap is
# lowered for the test, so it runs in a moment.)
extract.MAX_UNPACKED = 1 << 20
bomb = gzip.compress(b"\x00" * (8 << 20))
try:
    extract._gunzip(io.BytesIO(bomb), "db.gz")
    check("gzip bomb refused", False, "unpacked")
except ValueError as e:
    check("gzip bomb refused", "unpacks to more than" in str(e), e)

# Zips nested past MAX_NESTED_ZIPS: not followed.
blob = gzip.compress(good)
z = io.BytesIO()
with zipfile.ZipFile(z, "w") as zf:
    zf.writestr("db.gz", blob)
for _ in range(extract.MAX_NESTED_ZIPS + 2):
    outer = io.BytesIO()
    with zipfile.ZipFile(outer, "w") as zf:
        zf.writestr("inner.unf", z.getvalue())
    z = outer
with zipfile.ZipFile(io.BytesIO(z.getvalue())) as zf:
    check("zips nested too deep aren't followed", extract._find_db_gz_bytes(zf) is None)

print(f"\n{'all passed' if not failed else f'{failed} failed'}")
sys.exit(1 if failed else 0)
