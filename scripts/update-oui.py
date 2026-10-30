#!/usr/bin/env python3
"""Rebuild assets/oui.txt.gz, the MAC vendor table bundled into is-alive.

Source is the IEEE registry (MA-L, MA-M and MA-S) as mirrored by arp-scan's
ieee-oui.txt, one "<hex prefix>\t<vendor>" per line. Vendor names are
trimmed of corporate suffixes to keep the table small and the UI readable.

    python3 scripts/update-oui.py [ieee-oui.txt]

With no argument the file is downloaded from arp-scan's repository.
"""
import gzip
import re
import sys
import urllib.request

URL = "https://raw.githubusercontent.com/royhills/arp-scan/master/ieee-oui.txt"
OUT = "assets/oui.txt.gz"
SUFFIX = re.compile(
    r"[\s,.]+(inc|incorporated|ltd|limited|llc|l\.l\.c|corp|corporation|co|company|"
    r"gmbh|ag|sa|s\.a|s\.p\.a|spa|bv|b\.v|ab|oy|as|a/s|plc|pty|kk|k\.k|srl|s\.r\.l|"
    r"sas|nv|n\.v|technologies|technology)\.?$",
    re.I,
)


def short(name):
    name = " ".join(name.split())
    while True:
        trimmed = SUFFIX.sub("", name).rstrip(" ,.")
        if trimmed == name or not trimmed:
            return name
        name = trimmed


def main():
    if len(sys.argv) > 1:
        text = open(sys.argv[1], encoding="utf-8", errors="replace").read()
    else:
        text = urllib.request.urlopen(URL).read().decode("utf-8", "replace")
    rows = {}
    for line in text.splitlines():
        if not line or line.startswith("#") or "\t" not in line:
            continue
        prefix, vendor = line.split("\t", 1)
        prefix = prefix.strip().upper()
        if len(prefix) in (6, 7, 9) and all(c in "0123456789ABCDEF" for c in prefix):
            rows[prefix] = short(vendor)
    body = "".join(f"{p}\t{v}\n" for p, v in sorted(rows.items()))
    with gzip.GzipFile(OUT, "wb", mtime=0) as f:
        f.write(body.encode())
    print(f"{OUT}: {len(rows)} prefixes")


if __name__ == "__main__":
    main()
