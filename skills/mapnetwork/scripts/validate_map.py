#!/usr/bin/env python3
"""
validate_map.py — structural validator for /mapnetwork's .dc.html output.

Step 3.5 of the /mapnetwork workflow: run against every map build or update
before the file is delivered. Read-only — it never modifies the map, and it
never looks at rendering, theming, or anything outside the SITE DATA block.

Usage:
    python3 scripts/validate_map.py <path-to-map>.dc.html

Exit codes:
    0  valid (warnings may still be printed)
    1  errors found, or the SITE DATA block could not be read/parsed
    2  usage error (bad arguments, file not found)

How it reads the data
---------------------
The SITE DATA block is a run of `this.NAME = <JS literal>;` statements. A
single tokenizer walks the block once, tracking strings and comments, and
re-emits each value as canonical JSON which is then loaded with json.loads.
One pass matters: an earlier version ran separate regex passes for comments,
unquoted keys, quote style and trailing commas, and each of those passes
misfired on text *inside* strings (a URL's "//", "mgmt: .5" in a details
line, an apostrophe inside a /* */ comment). Only the tokenizer sees string
and comment boundaries; nothing downstream of it has to.

Accepted syntax is exactly what the schema uses: object/array literals,
single- or double-quoted strings, numbers, true/false/null, unquoted keys,
trailing commas, // and /* */ comments. Anything else (template literals,
`undefined`, function calls, variables) is rejected with the source line
number, because the check can't be trusted on data it can't read.

What it checks — every check maps to a specific engine behavior
----------------------------------------------------------------
Errors (the engine crashes, silently drops data, or renders wrong):
  - META, ZONES, NODES, EDGES, ACTIONS present with the right type
    (the engine dereferences all five without a default; SITES and BANDS
    are optional).
  - Every node has an id and a zone; node ids are unique.
  - Every zone defines both `dark` and `light` colours (custom zones take
    their colour straight from these; a missing one renders undefined).
  - Every edge has an id, ids are unique (the site-collapse trunk logic
    keys its hide/redirect sets by edge id), and from/to resolve to nodes
    (the engine drops unresolved edges with only a console warning).
  - No self-loops and no directed cycles of any length. Edges mean
    upstream -> downstream; a cycle is a contradiction in the data and
    removes the "no incoming edge" root the auto-layout starts from. A
    two-node cycle gets its own message: it is the reciprocal-uplink
    pattern /unifidump's `signals.reciprocal_uplinks` reports upstream.
  - Every action has a unique id (highlight state keys on it), a `sev` of
    high/med/low (anything else renders an uncoloured dot and is left out
    of the findings-band counts), and node refs that resolve.
  - Node zone refs exist in ZONES; node site refs exist in SITES when SITES
    is non-empty (both fall back silently to grey "Unzoned"/"Unassigned").
  - Node count over the 120 hard cap.

Warnings (renders, but not as authored):
  - `kind` over 18 characters (SKILL.md authoring cap).
  - Node with no `site` on a map that declares SITES (lands in
    "Unassigned" with no console warning at all).
  - Edge `cls` other than backbone/wired/wifi (draws with wifi weight).
  - Node count over the 90 target.

Out of scope: palette adherence, finding quality, severity honesty, client
inclusion, layout aesthetics. Those are Step 3 judgment, not structure.
"""

import json
import re
import sys
from pathlib import Path

NODE_KIND_MAX_LEN = 18
NODE_BUDGET_HARD_CAP = 120
NODE_BUDGET_TARGET = 90
VALID_SEV = ("high", "med", "low")
VALID_CLS = ("backbone", "wired", "wifi")

SITE_DATA_START_RE = re.compile(r"/\*\s*=+\s*SITE DATA\s*=+")
SITE_DATA_END_RE = re.compile(r"/\*\s*=+\s*END SITE DATA")

_IDENT_START = re.compile(r"[A-Za-z_$]")
_IDENT = re.compile(r"[A-Za-z_$][A-Za-z0-9_$]*")
_NUMBER = re.compile(r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?")
_SIMPLE_ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "b": "\b", "f": "\f",
                   "v": "\v", "0": "\0"}


class SiteDataError(ValueError):
    """Unreadable SITE DATA. Message carries a line number where possible."""


_APOS_IN_WORD = re.compile(r"[A-Za-z]'[A-Za-z]")


def _hint(line_text):
    """Most hand-edit parse failures in practice are a possessive or
    contraction inside a single-quoted string ('Elaine's Office'). When the
    failing line has that shape, say so."""
    if line_text and _APOS_IN_WORD.search(line_text):
        return (" — this line has an apostrophe inside a word; if it sits in "
                "a '...'-quoted string, switch that string to \"...\" quotes")
    return ""


# --------------------------------------------------------------------------
# Extraction
# --------------------------------------------------------------------------

def extract_site_data_block(html):
    """Return (block_text, first_line_number) for the code between the SITE
    DATA banner comment and the END SITE DATA marker, banner removed."""
    start = SITE_DATA_START_RE.search(html)
    if not start:
        raise SiteDataError("SITE DATA start marker not found — is this a "
                            "/mapnetwork .dc.html file?")
    end = SITE_DATA_END_RE.search(html, start.end())
    if not end:
        raise SiteDataError("END SITE DATA marker not found — file may be "
                            "truncated.")
    # Hand the tokenizer the banner comment itself, starting at its `/*`, and
    # let it find the real closing `*/`. Never search for `*/` here: if a
    # splice dropped the banner's closer, the whole block is commented out in
    # the browser, and any fallback (or a `*/` from a later comment in the
    # data) would parse data the engine never executes. Starting at the
    # banner's `/*` makes that case an "unterminated /* comment" error.
    body_start = start.start()
    first_line = html.count("\n", 0, body_start) + 1
    return html[body_start:end.start()], first_line


# --------------------------------------------------------------------------
# Tokenizer — the only code that knows where strings and comments are
# --------------------------------------------------------------------------

def tokenize(src, line_offset=1):
    """Yield (kind, value, line) tokens. kind is one of
    'str', 'num', 'ident', 'punct'. Comments and whitespace are dropped."""
    tokens = []
    i, n, line = 0, len(src), line_offset
    while i < n:
        c = src[i]
        if c == "\n":
            line += 1; i += 1; continue
        if c in " \t\r\ufeff":
            i += 1; continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j == -1 else j
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            if j == -1:
                raise SiteDataError(
                    f"line {line}: unterminated /* comment — if this is the "
                    f"SITE DATA banner, the data was spliced inside it and is "
                    f"commented out; splice after the banner's closing */")
            line += src.count("\n", i, j)
            i = j + 2
            continue
        if c in "'\"":
            start_line = line
            quote, i, buf = c, i + 1, []
            while True:
                if i >= n:
                    raise SiteDataError(f"line {start_line}: unterminated string")
                ch = src[i]
                if ch == quote:
                    i += 1; break
                if ch == "\n":
                    raise SiteDataError(
                        f"line {start_line}: string runs past end of line — "
                        f"likely an unescaped {quote} inside a {quote}-quoted "
                        f"string (switch the outer quote style)")
                if ch == "\\":
                    if i + 1 >= n:
                        raise SiteDataError(f"line {line}: dangling backslash")
                    esc = src[i + 1]
                    if esc == "u" and re.fullmatch(r"[0-9a-fA-F]{4}", src[i + 2:i + 6]):
                        buf.append(chr(int(src[i + 2:i + 6], 16))); i += 6; continue
                    if esc == "\n":          # line continuation
                        line += 1; i += 2; continue
                    buf.append(_SIMPLE_ESCAPES.get(esc, esc)); i += 2; continue
                buf.append(ch); i += 1
            tokens.append(("str", "".join(buf), start_line))
            continue
        if c == "`":
            raise SiteDataError(f"line {line}: template literals (`...`) are "
                                f"not supported in SITE DATA — use quotes")
        m = _NUMBER.match(src, i)
        if m and (c.isdigit() or (c == "-" and i + 1 < n and src[i + 1].isdigit())):
            tokens.append(("num", m.group(0), line)); i = m.end(); continue
        if _IDENT_START.match(c):
            m = _IDENT.match(src, i)
            tokens.append(("ident", m.group(0), line)); i = m.end(); continue
        if c in "{}[]:,=;.":
            tokens.append(("punct", c, line)); i += 1; continue
        raise SiteDataError(f"line {line}: unexpected character {c!r}"
                            + _hint(src.splitlines()[line - line_offset]
                                    if line - line_offset < len(src.splitlines()) else ""))
    return tokens


# --------------------------------------------------------------------------
# Statement splitting + JSON emission
# --------------------------------------------------------------------------

def split_assignments(tokens):
    """Split the token stream into [(name, value_tokens, line)] for each
    top-level `this.NAME = value[;]` statement."""
    out, i, n = [], 0, len(tokens)
    while i < n:
        t = tokens[i]
        if t[:2] == ("punct", ";"):
            i += 1; continue
        if not (i + 3 < n and t[:2] == ("ident", "this")
                and tokens[i + 1][:2] == ("punct", ".")
                and tokens[i + 2][0] == "ident"
                and tokens[i + 3][:2] == ("punct", "=")):
            raise SiteDataError(f"line {t[2]}: expected `this.NAME = ...`, "
                                f"found {t[1]!r}")
        name, line = tokens[i + 2][1], t[2]
        i += 4
        stack, start = [], i          # (opener, line) — pinpoints mismatches
        pairs = {"}": "{", "]": "["}
        while i < n:
            kind, val, tline = tokens[i]
            if kind == "punct" and val in "{[":
                stack.append((val, tline))
            elif kind == "punct" and val in "}]":
                if not stack:
                    raise SiteDataError(f"line {tline}: `{val}` has nothing to close")
                opener, oline = stack.pop()
                if opener != pairs[val]:
                    raise SiteDataError(
                        f"line {tline}: `{val}` closes a `{opener}` opened on "
                        f"line {oline} — missing `{'}' if opener == '{' else ']'}`")
            elif not stack and ((kind == "punct" and val == ";") or
                                (kind == "ident" and val == "this")):
                break
            i += 1
        if stack:
            opener, oline = stack[-1]
            raise SiteDataError(f"line {oline}: `{opener}` in `this.{name}` is "
                                f"never closed")
        out.append((name, tokens[start:i], line))
    return out


def tokens_to_json(tokens):
    """Emit canonical JSON for one value's tokens. Returns (json_text,
    offsets) where offsets[k] is the char offset where token k starts, so a
    json.loads error position can be mapped back to a source line."""
    parts, offsets, pos = [], [], 0
    for k, (kind, val, line) in enumerate(tokens):
        nxt = tokens[k + 1] if k + 1 < len(tokens) else None
        if kind == "str":
            s = json.dumps(val, ensure_ascii=False)
        elif kind == "num":
            s = val
        elif kind == "ident":
            if nxt and nxt[:2] == ("punct", ":"):
                s = json.dumps(val)                       # unquoted key
            elif val in ("true", "false", "null"):
                s = val
            else:
                raise SiteDataError(
                    f"line {line}: `{val}` is not a literal value (only "
                    f"strings, numbers, true/false/null, objects, arrays)")
        else:  # punct
            if val == "," and nxt and nxt[:2] in (("punct", "}"), ("punct", "]")):
                s = ""                                     # trailing comma
            elif val in "{}[]:,":
                s = val
            else:
                raise SiteDataError(f"line {line}: unexpected {val!r} inside a value")
        offsets.append(pos)
        parts.append(s)
        pos += len(s)
    return "".join(parts), offsets


def parse_site_data(html):
    """Return {NAME: python_value} for every assignment in SITE DATA."""
    block, first_line = extract_site_data_block(html)
    block_lines = block.splitlines()
    result = {}
    for name, toks, line in split_assignments(tokenize(block, first_line)):
        if not toks:
            raise SiteDataError(f"line {line}: `this.{name} =` has no value")
        text, offsets = tokens_to_json(toks)
        try:
            result[name] = json.loads(text)
        except json.JSONDecodeError as e:
            k = max((j for j, off in enumerate(offsets) if off <= e.pos), default=0)
            src_line = toks[min(k, len(toks) - 1)][2]
            raise SiteDataError(
                f"line {src_line}: `this.{name}` is not valid data ({e.msg}) — "
                f"usually a missing comma between properties or a stray "
                f"bracket near this line"
                + _hint(block_lines[src_line - first_line]
                        if 0 <= src_line - first_line < len(block_lines) else "")) from None
    return result


# --------------------------------------------------------------------------
# Checks
# --------------------------------------------------------------------------

def _find_cycles(node_ids, edges):
    """Return a list of directed cycles, each a list of node ids, using an
    iterative DFS (no recursion limit on large maps). Self-loops excluded —
    reported separately."""
    adj = {nid: [] for nid in node_ids}
    for e in edges:
        f, t = e.get("from"), e.get("to")
        if f in adj and t in adj and f != t:
            adj[f].append(t)
    WHITE, GREY, BLACK = 0, 1, 2
    color = {nid: WHITE for nid in node_ids}
    cycles, seen = [], set()
    for root in node_ids:
        if color[root] != WHITE:
            continue
        stack, path = [(root, iter(adj[root]))], [root]
        color[root] = GREY
        while stack:
            node, it = stack[-1]
            advanced = False
            for nxt in it:
                if color[nxt] == GREY:
                    cyc = path[path.index(nxt):]
                    key = frozenset(cyc)
                    if key not in seen:
                        seen.add(key); cycles.append(cyc)
                elif color[nxt] == WHITE:
                    color[nxt] = GREY
                    stack.append((nxt, iter(adj[nxt]))); path.append(nxt)
                    advanced = True
                    break
            if not advanced:
                color[node] = BLACK
                stack.pop(); path.pop()
    return cycles


def validate(data):
    """Return (errors, warnings) for parsed SITE DATA."""
    errors, warnings = [], []

    expected = {"META": dict, "ZONES": dict, "NODES": list,
                "EDGES": list, "ACTIONS": list}
    for key, typ in expected.items():
        if key not in data:
            errors.append(f"`this.{key}` is missing (the engine has no default for it).")
        elif not isinstance(data[key], typ):
            errors.append(f"`this.{key}` must be {'an object' if typ is dict else 'an array'}.")
    if "SITES" in data and not isinstance(data["SITES"], dict):
        errors.append("`this.SITES` must be an object.")

    def get(key, typ):
        v = data.get(key)
        return v if isinstance(v, typ) else typ()

    zones, nodes = get("ZONES", dict), get("NODES", list)
    edges, actions, sites = get("EDGES", list), get("ACTIONS", list), get("SITES", dict)

    # ---- zones ----
    for zk, z in zones.items():
        if not isinstance(z, dict):
            errors.append(f"Zone '{zk}' is not an object."); continue
        missing = [c for c in ("dark", "light") if not z.get(c)]
        if missing:
            errors.append(f"Zone '{zk}' has no {'/'.join(missing)} colour.")

    # ---- nodes ----
    node_ids, dupes = [], set()
    for idx, nd in enumerate(nodes):
        if not isinstance(nd, dict):
            errors.append(f"NODES[{idx}] is not an object."); continue
        nid = nd.get("id")
        if not nid:
            errors.append(f"NODES[{idx}] has no id."); continue
        if nid in node_ids:
            dupes.add(nid)
        node_ids.append(nid)
        zone = nd.get("zone")
        if not zone:
            errors.append(f"Node '{nid}' has no zone.")
        elif zone not in zones:
            errors.append(f"Node '{nid}': zone '{zone}' is not in ZONES "
                          f"(renders as grey 'Unzoned').")
        site = nd.get("site")
        if sites:
            if site is None:
                warnings.append(f"Node '{nid}' has no site on a map that declares "
                                f"SITES — it will land in 'Unassigned'.")
            elif site not in sites:
                errors.append(f"Node '{nid}': site '{site}' is not in SITES "
                              f"(renders under 'Unassigned').")
        kind = nd.get("kind")
        if isinstance(kind, str) and len(kind) > NODE_KIND_MAX_LEN:
            warnings.append(f"Node '{nid}': kind '{kind}' is {len(kind)} chars "
                            f"(cap {NODE_KIND_MAX_LEN}).")
    for nid in sorted(dupes):
        errors.append(f"Duplicate node id '{nid}' ({node_ids.count(nid)}x).")
    id_set = set(node_ids)

    # ---- edges ----
    edge_ids, edup = [], set()
    for idx, e in enumerate(edges):
        if not isinstance(e, dict):
            errors.append(f"EDGES[{idx}] is not an object."); continue
        eid = e.get("id")
        label = eid if eid else f"EDGES[{idx}]"
        if not eid:
            errors.append(f"{label} has no id.")
        else:
            if eid in edge_ids:
                edup.add(eid)
            edge_ids.append(eid)
        f, t = e.get("from"), e.get("to")
        if f not in id_set:
            errors.append(f"Edge '{label}': from '{f}' is not a node (edge is dropped).")
        if t not in id_set:
            errors.append(f"Edge '{label}': to '{t}' is not a node (edge is dropped).")
        if f is not None and f == t:
            errors.append(f"Edge '{label}' is a self-loop on '{f}'.")
        cls = e.get("cls")
        if cls not in VALID_CLS:
            warnings.append(f"Edge '{label}': cls {cls!r} is not "
                            f"backbone/wired/wifi (draws with wifi weight).")
    for eid in sorted(edup):
        errors.append(f"Duplicate edge id '{eid}' ({edge_ids.count(eid)}x) — "
                      f"breaks site-collapse trunk folding.")

    for cyc in _find_cycles(list(dict.fromkeys(node_ids)), [e for e in edges if isinstance(e, dict)]):
        if len(cyc) == 2:
            a, b = cyc
            errors.append(f"Reciprocal edges '{a}' <-> '{b}': collapse to one "
                          f"directed edge (resolve by model size class, or flag "
                          f"for physical verification if same class).")
        else:
            errors.append("Directed cycle: " + " -> ".join(cyc + [cyc[0]]) +
                          " — edges must run upstream to downstream.")

    # ---- actions ----
    action_ids, adup = [], set()
    for idx, a in enumerate(actions):
        if not isinstance(a, dict):
            errors.append(f"ACTIONS[{idx}] is not an object."); continue
        aid = a.get("id")
        label = aid if aid else f"ACTIONS[{idx}]"
        if not aid:
            errors.append(f"{label} has no id.")
        else:
            if aid in action_ids:
                adup.add(aid)
            action_ids.append(aid)
        if a.get("sev") not in VALID_SEV:
            errors.append(f"Action '{label}': sev {a.get('sev')!r} is not "
                          f"high/med/low (uncoloured, left out of band counts).")
        refs = a.get("nodes", [])
        if not isinstance(refs, list):
            errors.append(f"Action '{label}': nodes must be an array.")
            refs = []
        for ref in refs:
            if ref not in id_set:
                errors.append(f"Action '{label}': node '{ref}' is not a node.")
    for aid in sorted(adup):
        errors.append(f"Duplicate action id '{aid}' ({action_ids.count(aid)}x).")

    # ---- budget ----
    count = len(nodes)
    if count > NODE_BUDGET_HARD_CAP:
        errors.append(f"{count} nodes exceeds the {NODE_BUDGET_HARD_CAP}-node hard cap.")
    elif count > NODE_BUDGET_TARGET:
        warnings.append(f"{count} nodes exceeds the {NODE_BUDGET_TARGET}-node target.")

    return errors, warnings


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------

def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 1:
        print("Usage: python3 validate_map.py <path-to-map>.dc.html", file=sys.stderr)
        return 2
    path = Path(argv[0])
    if not path.is_file():
        print(f"File not found: {path}", file=sys.stderr)
        return 2
    try:
        data = parse_site_data(path.read_text(encoding="utf-8"))
    except SiteDataError as e:
        print(f"PARSE FAILED: {e}")
        return 1
    except UnicodeDecodeError as e:
        print(f"PARSE FAILED: file is not UTF-8 ({e})")
        return 1

    errors, warnings = validate(data)

    def n(key):
        v = data.get(key)
        return len(v) if isinstance(v, (list, dict)) else 0
    print(f"{path.name}: NODES {n('NODES')} · EDGES {n('EDGES')} · "
          f"ACTIONS {n('ACTIONS')} · ZONES {n('ZONES')} · SITES {n('SITES')}")
    for w in warnings:
        print(f"  WARN: {w}")
    for e in errors:
        print(f"  ERR:  {e}")
    if errors:
        print(f"INVALID — {len(errors)} error(s), {len(warnings)} warning(s)")
        return 1
    print(f"VALID — 0 errors, {len(warnings)} warning(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
