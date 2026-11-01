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
MAC = re.compile(r"\b((?:[0-9a-fA-F]{2}:){3})((?:[0-9a-fA-F]{2}:){2}[0-9a-fA-F]{2})\b")


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
    def mac(m):
        key = m.group(0).lower()
        macs.setdefault(key, len(macs) + 1)
        return f"{m.group(1)}00:00:{macs[key]:02x}"
    return MAC.sub(mac, text)


def check(path, text, site_file):
    bad = []
    names = list(ORGS) + list(PLACES) + (list(SITE_ONLY) if site_file else [])
    for n in names:
        if re.search(rf"(?<![A-Za-z]){re.escape(n)}(?![a-z])", text, re.I):
            bad.append(f"name {n!r}")
    for m in MAC.finditer(text):
        if not m.group(2).lower().startswith("00:00:"):
            bad.append(f"MAC {m.group(0)}")
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
    out = Path(__file__).resolve().parent.parent / "skills"
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
            problems += check(dest.relative_to(out.parent), text, site_file)
            dest.write_text(text, encoding="utf-8")
    if problems:
        sys.exit("left in the copies:\n  " + "\n  ".join(problems))
    n = sum(1 for p in out.rglob("*") if p.is_file())
    print(f"skills/: {n} files, {len(macs)} MACs reduced to their vendor prefix")


if __name__ == "__main__":
    main()
