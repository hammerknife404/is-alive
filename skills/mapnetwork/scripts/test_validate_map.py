#!/usr/bin/env python3
"""
Regression suite for validate_map.py.

    python3 scripts/test_validate_map.py

Every case below is a defect found in code review, not a hypothetical:
the "must parse" group are valid maps an earlier version rejected or
misread; the "must flag" group are engine failure modes it missed; the
"must reject" group are broken files that have to fail with the right line.

If Node.js is on PATH, each parsed case is also deep-compared against what
the JS engine itself evaluates the SITE DATA block to — the strongest
available proof that the validator reads exactly what the browser reads.
Without Node, that comparison is skipped and everything else still runs.
"""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import validate_map as vm  # noqa: E402

ZONES = "this.ZONES = { core:{ name:'Core', vlan:'x', dark:'#5b9dff', light:'#2563c9' } };"


def page(body):
    return ("<script>\n/* ===================== SITE DATA =====================\n"
            "   test fixture\n   ==== */\n" + body +
            "\n/* =================== END SITE DATA =================== */\n</script>")


def site(nodes, edges="[ { id:'e1', from:'a', to:'b', cls:'wired' } ]",
         actions="[]", sites="{}", zones=ZONES):
    return page(f"this.META = {{ title:'T', subtitle:'s' }};\nthis.SITES = {sites};\n{zones}\n"
                f"this.NODES = {nodes};\nthis.EDGES = {edges};\n"
                f"this.ACTIONS = {actions};\nthis.BANDS = [];")


def two(details=""):
    return (f"[ {{ id:'a', zone:'core', kind:'X', label:'A', details:[{details}] }}, "
            f"{{ id:'b', zone:'core', kind:'X', label:'B' }} ]")


THREE = ("[ {id:'a',zone:'core',kind:'X',label:'A'},{id:'b',zone:'core',kind:'X',label:'B'},"
         "{id:'c',zone:'core',kind:'X',label:'C'} ]")

# (name, html, expected_exit, substring expected in output or None)
CASES = [
    # ---- valid maps that must parse, exit 0 ----
    ("url in string",            site(two("'https://portal.example.com/admin'")), 0, None),
    ("key-like text, dq string", site(two('"Two NICs, mgmt: .5"')), 0, None),
    ("key-like text, sq string", site(two("'Two NICs, mgmt: .5'")), 0, None),
    ("apostrophe in /* */",      site("[ /* Elaine's closet */ { id:'a', zone:'core', kind:'X', label:'A' },"
                                      " { id:'b', zone:'core', kind:'X', label:'B' } ]"), 0, None),
    ("apostrophe in //",         site("[ // Elaine's closet\n { id:'a', zone:'core', kind:'X', label:'A' },"
                                      " { id:'b', zone:'core', kind:'X', label:'B' } ]"), 0, None),
    ("dq string w/ apostrophe",  site(two('"Elaine\'s office"')), 0, None),
    ("escaped apostrophe",       site(two("'Elaine\\'s office'")), 0, None),
    ("', ]' inside string",      site(two("'ports 1-4, ]x'")), 0, None),
    ("'/*' inside string",       site(two("'fan 1/* tray'")), 0, None),
    ("'this.X =' in string",     site(two("'note: this.X = y'")), 0, None),
    ("semicolon in string",      site(two("'a; b'")), 0, None),
    ("trailing commas",          site("[ { id:'a', zone:'core', kind:'X', label:'A', }, "
                                      "{ id:'b', zone:'core', kind:'X', label:'B', }, ]"), 0, None),
    # ---- engine failure modes that must be flagged ----
    ("self-loop",       site(two(), "[ {id:'e1',from:'a',to:'b',cls:'wired'},{id:'e2',from:'b',to:'b',cls:'wired'} ]"), 1, "self-loop"),
    ("reciprocal pair", site(two(), "[ {id:'e1',from:'a',to:'b',cls:'wired'},{id:'e2',from:'b',to:'a',cls:'wired'} ]"), 1, "Reciprocal"),
    ("3-node cycle",    site(THREE, "[ {id:'e1',from:'a',to:'b',cls:'wired'},{id:'e2',from:'b',to:'c',cls:'wired'},"
                                    "{id:'e3',from:'c',to:'a',cls:'wired'} ]"), 1, "Directed cycle"),
    ("dup edge id",     site(two(), "[ {id:'e1',from:'a',to:'b',cls:'wired'},{id:'e1',from:'a',to:'b',cls:'wifi'} ]"), 1, "Duplicate edge id"),
    ("dup node id",     site("[ {id:'a',zone:'core',kind:'X',label:'A'},{id:'a',zone:'core',kind:'X',label:'B'} ]", "[]"), 1, "Duplicate node id"),
    ("dangling edge",   site(two(), "[ {id:'e1',from:'a',to:'zz',cls:'wired'} ]"), 1, "is not a node"),
    ("bad sev",         site(two(), actions="[ {id:'ai1',sev:'critical',text:'x',nodes:['a']} ]"), 1, "sev"),
    ("dup action id",   site(two(), actions="[ {id:'ai1',sev:'low',text:'x',nodes:[]},{id:'ai1',sev:'low',text:'y',nodes:[]} ]"), 1, "Duplicate action id"),
    ("dangling action", site(two(), actions="[ {id:'ai1',sev:'low',text:'x',nodes:['zz']} ]"), 1, "is not a node"),
    ("bad zone ref",    site("[ {id:'a',zone:'nope',kind:'X',label:'A'},{id:'b',zone:'core',kind:'X',label:'B'} ]"), 1, "not in ZONES"),
    ("zone no colours", site(two(), zones="this.ZONES = { core:{ name:'Core' } };"), 1, "colour"),
    ("bad site ref",    site("[ {id:'a',site:'x',zone:'core',kind:'X',label:'A'},{id:'b',site:'hq',zone:'core',kind:'X',label:'B'} ]",
                             sites="{ hq:{ name:'HQ' } }"), 1, "not in SITES"),
    ("missing ACTIONS", page("this.META = { title:'T' };\nthis.ZONES = {};\nthis.NODES = [];\nthis.EDGES = [];"), 1, "ACTIONS` is missing"),
    ("over hard cap",   site("[" + ",".join(f"{{id:'n{i}',zone:'core',kind:'X',label:'N'}}" for i in range(121)) + "]", "[]"), 1, "hard cap"),
    # ---- warnings (exit 0) ----
    ("kind > 18",       site("[ {id:'a',zone:'core',kind:'SONICWALL (INFERRED)',label:'A'},{id:'b',zone:'core',kind:'X',label:'B'} ]"), 0, "20 chars"),
    ("unknown cls",     site(two(), "[ {id:'e1',from:'a',to:'b',cls:'wireless'} ]"), 0, "cls"),
    ("node w/o site",   site("[ {id:'a',site:'hq',zone:'core',kind:'X',label:'A'},{id:'b',zone:'core',kind:'X',label:'B'} ]",
                             sites="{ hq:{ name:'HQ' } }"), 0, "Unassigned"),
    # ---- broken files: must reject, at the right line ----
    ("missing comma",     site("[ { id:'a' zone:'core' } ]"), 1, "line 8"),
    ("unescaped apos",    site(two("'Elaine's office'")), 1, "unescaped"),
    ("mismatched close",  site("[ { id:'a', chips:['x' } ]"), 1, "closes a `[`"),
    ("unterminated str",  site(two("'oops")), 1, "string"),
    ("undefined literal", site("[ { id:'a', zone:'core', flags:undefined } ]"), 1, "not a literal"),
    ("template literal",  site(two("`x`")), 1, "template"),
    ("no markers",        "<html>nothing here</html>", 1, "marker"),
    # ASC of Northern LA, 2026-10-05: data spliced after the banner's first
    # line, so the banner's */ was dropped and every this.X = ... sat inside
    # the comment. Browser rendered undefined data; validator said VALID.
    ("unclosed banner",
     site(two()).replace("   test fixture\n   ==== */\n", ""), 1, "commented out"),
    # Same splice, but the data opens with its own /* */ comment. JS comments
    # don't nest: that */ closes the banner and the data after it executes.
    # Must parse (and the oracle confirms the browser agrees).
    ("unclosed banner, inner comment closes it",
     site(two()).replace("   test fixture\n   ==== */\n", "/* note */\n"), 0, None),
]

ORACLE_JS = r"""
const h=require('fs').readFileSync(process.argv[1],'utf8');
const s=h.search(/\/\*\s*=+\s*SITE DATA\s*=+/), e=h.search(/\/\*\s*=+\s*END SITE DATA/);
const ctx={}; new Function(h.slice(s,e)).call(ctx); process.stdout.write(JSON.stringify(ctx));
"""


def run():
    node = shutil.which("node")
    failures, oracle_checked = [], 0
    with tempfile.TemporaryDirectory() as tmp:
        for name, html, want_exit, want_text in CASES:
            path = Path(tmp) / "case.dc.html"
            path.write_text(html, encoding="utf-8")
            proc = subprocess.run([sys.executable, vm.__file__, str(path)],
                                  capture_output=True, text=True)
            out = proc.stdout + proc.stderr
            ok = proc.returncode == want_exit and (want_text is None or want_text in out)
            if ok and node and want_exit == 0:
                js = subprocess.run([node, "-e", ORACLE_JS, str(path)],
                                    capture_output=True, text=True)
                if js.returncode == 0:
                    oracle_checked += 1
                    if json.loads(js.stdout) != vm.parse_site_data(html):
                        ok, out = False, "parsed data differs from JS engine evaluation"
            print(f"{'PASS' if ok else 'FAIL'}  {name}")
            if not ok:
                failures.append((name, out.strip()))
    print(f"\n{len(CASES) - len(failures)}/{len(CASES)} passed"
          + (f"; {oracle_checked} parsed cases deep-matched the JS engine"
             if node else "; Node.js not found, JS-engine comparison skipped"))
    for name, out in failures:
        print(f"\n--- {name} ---\n{out}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(run())
