#!/usr/bin/env python3
"""Rebuild assets/oui.txt.gz, the MAC vendor table bundled into is-alive.

Source is the IEEE registry (MA-L, MA-M and MA-S) as mirrored by arp-scan's
ieee-oui.txt, one "<hex prefix>\t<vendor>" per line. Vendor names are
trimmed of corporate suffixes to keep the table small and the UI readable.

    python3 scripts/update-oui.py [ieee-oui.txt]

With no argument the file is downloaded from arp-scan's repository.

assets/oui.date holds the day the table last changed, which Settings
shows. It is rewritten only when the table's contents differ, so a run
that finds nothing new leaves no diff (the weekly workflow opens a pull
request only when there is one).
"""
import datetime
import gzip
import re
import sys
import urllib.request

URL = "https://raw.githubusercontent.com/royhills/arp-scan/master/ieee-oui.txt"
OUT = "assets/oui.txt.gz"
DATE = "assets/oui.date"
# Fewer rows than this is a broken download, not a registry: keep the old table.
MIN_ROWS = 30000
SUFFIX = re.compile(
    r"[\s,.]+(inc|incorporated|ltd|limited|llc|l\.l\.c|corp|corporation|co|company|"
    r"gmbh|ag|sa|s\.a|s\.p\.a|spa|bv|b\.v|ab|oy|as|a/s|plc|pty|kk|k\.k|srl|s\.r\.l|"
    r"sas|nv|n\.v|technologies|technology)\.?$",
    re.I,
)


# What a vendor name can't hold: control and Unicode format (bidi,
# zero-width) characters, and markup. The table is downloaded each week.
BAD = re.compile(r"[\x00-\x1f\x7f-\x9f<>\u00ad\u200b-\u200f\u202a-\u202e\u2060-\u2064\u2066-\u2069\ufeff]")


def short(name):
    name = " ".join(BAD.sub("", name).split())[:80]
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
    if len(rows) < MIN_ROWS:
        sys.exit(f"only {len(rows)} prefixes read: not replacing {OUT}")
    body = "".join(f"{p}\t{v}\n" for p, v in sorted(rows.items())).encode()
    try:
        with gzip.open(OUT, "rb") as f:
            old = f.read()
    except OSError:
        old = b""
    if body == old:
        print(f"{OUT}: {len(rows)} prefixes, unchanged")
        return
    with gzip.GzipFile(OUT, "wb", mtime=0) as f:
        f.write(body)
    today = datetime.datetime.now(datetime.timezone.utc).date().isoformat()
    with open(DATE, "w", encoding="ascii") as f:
        f.write(today + "\n")
    # What changed, for the pull request a reviewer reads before merging.
    before = dict(
        line.split("\t", 1) for line in old.decode("utf-8", "replace").splitlines() if "\t" in line
    )
    added = sum(1 for p in rows if p not in before)
    removed = sum(1 for p in before if p not in rows)
    renamed = sum(1 for p, v in rows.items() if p in before and before[p] != v)
    print(f"{OUT}: {len(rows)} prefixes, changed ({added} added, {renamed} renamed, "
          f"{removed} removed); {DATE}: {today}")


if __name__ == "__main__":
    main()
