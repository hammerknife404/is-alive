#!/usr/bin/env python3
"""Copy the /unifidump and /mapnetwork skills into skills/, scrubbed.

    python3 scripts/sync-skills.py ~/.claude/skills/synced/<id>

The copies ship inside every Site Bundle (for a Claude chat that doesn't have
the skills installed), so they're public: this repo and every release carry
them. Names that may be real sites become fictional ones, MACs keep only their
vendor prefix, and the run fails if anything identifying is left. The
replacement table is local (dev-only/skills-scrub.json), not in the repo.
"""
import ipaddress, json, re, shutil, sys
from pathlib import Path

SKILLS = ["unifidump", "mapnetwork"]
# The replacements name what they hide, so they live outside the repo:
# dev-only/ is gitignored. Keys: orgs, places (old -> new), site_only (renamed
# only in the example maps, e.g. a place that is also a vendor's name) and
# fixed (exact strings: a serial, an address).
TABLE = Path(__file__).resolve().parent.parent / "dev-only" / "skills-scrub.json"
_t = json.loads(TABLE.read_text(encoding="utf-8")) if TABLE.exists() else None
ORGS = _t["orgs"] if _t else {}
PLACES = _t["places"] if _t else {}
SITE_ONLY = _t["site_only"] if _t else {}
FIXED = _t["fixed"] if _t else {}
DOC_NETS = [ipaddress.ip_network(n) for n in ("192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24")]
# A MAC is written many ways: colon or dash pairs, Cisco's dotted quads, a
# SonicWall serial's dashed quads (the serial is the X0 MAC), and bare (an id
# derived from a MAC, d_<12 hex>). Each is a run of hex and separators with
# one of these shapes; the same rule is in src/skills.rs's test.
RUN = re.compile(r"[0-9A-Fa-f:.-]+")
SHAPES = re.compile(
    r"(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}"
    r"|[0-9A-Fa-f]{4}[.-][0-9A-Fa-f]{4}[.-][0-9A-Fa-f]{4}"
    r"|[0-9A-Fa-f]{12}"
)
# Not a device: the format example in validate_map.py's warning.
PLACEHOLDERS = {"aabbccddeeff"}


def macs_in(text):
    """(start, end, 12 lowercase hex) for each MAC in text, in any form."""
    for m in RUN.finditer(text):
        run = m.group(0)
        core = run.strip(":.-")
        at = m.start() + run.index(core) if core else m.start()
        if not SHAPES.fullmatch(core):
            continue
        hexes = re.sub(r"[^0-9A-Fa-f]", "", core).lower()
        # Twelve bare digits are a date or a number more often than a MAC.
        if core == hexes and not re.search(r"[a-f]", hexes):
            continue
        yield at, at + len(core), hexes


def forms(old, new):
    """Title, lower, UPPER, and snake/kebab forms of a (multi-word) name."""
    out = {old: new, old.lower(): new.lower(), old.upper(): new.upper()}
    if " " in old:
        for sep in "_-":
            out[old.replace(" ", sep)] = new.replace(" ", sep)
            out[old.lower().replace(" ", sep)] = new.lower().replace(" ", sep)
    return out


def table(site_file):
    t = {}
    for old, new in {**ORGS, **PLACES, **(SITE_ONLY if site_file else {})}.items():
        t.update(forms(old, new))
    t.update(FIXED)
    return t


def scrub(text, site_file, macs):
    t = table(site_file)
    for old in sorted(t, key=len, reverse=True):  # longest first: "X Clinic" before "X"
        text = re.sub(rf"(?<![A-Za-z]){re.escape(old)}(?![a-z])", t[old], text)
    # Every form of one MAC gets the same number, so an id stays the MAC
    # it was made from: d_10da43f1165d and 10:da:43:f1:16:5d both end 00:02.
    out, last = [], 0
    for start, end, hexes in macs_in(text):
        if hexes[6:10] == "0000" or hexes in PLACEHOLDERS:
            continue
        n = macs.setdefault(hexes, len(macs) + 1)
        if n > 0xFF:
            sys.exit("more than 255 MACs: widen the numbering in scrub()")
        keep = iter(f"0000{n:02x}")
        seen, chars = 0, []
        for c in text[start:end]:
            if c in "0123456789abcdefABCDEF":
                seen += 1
                if seen > 6:
                    c = next(keep)
                    c = c.upper() if text[start:end].isupper() else c
            chars.append(c)
        out += [text[last:start], "".join(chars)]
        last = end
    return "".join(out) + text[last:]


def check(path, text, site_file):
    bad = []
    names = list(ORGS) + list(PLACES) + (list(SITE_ONLY) if site_file else [])
    for n in names:
        if re.search(rf"(?<![A-Za-z]){re.escape(n)}(?![a-z])", text, re.I):
            bad.append(f"name {n!r}")
    for start, end, hexes in macs_in(text):
        if hexes[6:10] != "0000" and hexes not in PLACEHOLDERS:
            bad.append(f"MAC {text[start:end]}")
    for ip in re.findall(r"\b(?:\d{1,3}\.){3}\d{1,3}\b", text):
        try:
            a = ipaddress.ip_address(ip)
        except ValueError:
            continue
        if a.is_global and not any(a in n for n in DOC_NETS):
            bad.append(f"public IP {ip}")
    return [f"{path}: {b}" for b in bad]


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    if _t is None:
        sys.exit(f"no scrub table at {TABLE}: the names to replace aren't kept in the repo")
    src = Path(sys.argv[1]).expanduser()
    final = Path(__file__).resolve().parent.parent / "skills"
    # Built beside skills/ and swapped in only when nothing is flagged: a
    # failed run never leaves unscrubbed text where `git add` would take it.
    out = final.with_name("skills.new")
    shutil.rmtree(out, ignore_errors=True)
    macs, problems = {}, []
    for skill in SKILLS:
        for f in sorted((src / skill).rglob("*")):
            if not f.is_file() or "__pycache__" in f.parts:
                continue
            rel = f.relative_to(src)
            site_file = "example-" in f.name
            dest = out / scrub(str(rel), site_file, macs)
            dest.parent.mkdir(parents=True, exist_ok=True)
            text = scrub(f.read_text(encoding="utf-8"), site_file, macs)
            problems += check(final.name / dest.relative_to(out), text, site_file)
            dest.write_text(text, encoding="utf-8")
    if problems:
        shutil.rmtree(out, ignore_errors=True)
        sys.exit("left in the copies (skills/ unchanged):\n  " + "\n  ".join(problems))
    old = final.with_name("skills.old")
    shutil.rmtree(old, ignore_errors=True)
    if final.exists():
        final.rename(old)
    out.rename(final)
    shutil.rmtree(old, ignore_errors=True)
    n = sum(1 for p in final.rglob("*") if p.is_file())
    print(f"skills/: {n} files, {len(macs)} MACs reduced to their vendor prefix")


if __name__ == "__main__":
    main()
