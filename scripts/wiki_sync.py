#!/usr/bin/env python3
"""Merge docs/wiki into a clone of the GitHub wiki, page by page.

The wiki's pages live in the repository (docs/wiki), so a change and the
pages it changes ship in one pull request. A cloud session can't push to the
wiki itself (a wiki isn't a repository the Claude GitHub App can be given),
so .github/workflows/wiki.yml runs this on main and pushes the result.

What changed in docs/wiki since the last sync (the commit named in the
wiki's newest "Synced-From:" trailer) is merged into the wiki's own copy:

- a page changed only here is written to the wiki (or removed from it);
- a page changed only on the wiki is left as it is;
- a page changed in both is merged line by line (git merge-file), and where
  the two touch the same lines it's a clash: nothing is written at all, and
  the run says which pages to bring into docs/wiki first;
- a page only the wiki has is left alone.

The first sync has no trailer to go by. The wiki must then still be at the
commit docs/wiki was taken from (--imported-from), so nothing on it is lost.

Exit 0 with a new commit in the wiki clone (or nothing to do); 1 on a clash
or a wiki that moved before its first sync, with the clone untouched.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile

TRAILER = "Synced-From"


def git(cwd, *args):
    """Run git in `cwd`; its output as bytes."""
    return subprocess.run(
        ["git", "-C", cwd, *args], check=True, capture_output=True
    ).stdout


def tree(repo, rev, sub=None):
    """Every file at `rev` (under `sub`), by its path relative to `sub`."""
    args = ["ls-tree", "-r", "-z", "--name-only", rev]
    if sub:
        args += ["--", sub]
    out = {}
    for raw in git(repo, *args).split(b"\0"):
        if not raw:
            continue
        name = raw.decode()
        rel = os.path.relpath(name, sub) if sub else name
        out[rel] = git(repo, "cat-file", "blob", f"{rev}:{name}")
    return out


def last_sync(wiki):
    """The repository commit the wiki was last synced from, if it ever was."""
    body = git(wiki, "log", "-1", f"--grep=^{TRAILER}: ", "--format=%B").decode()
    m = re.search(rf"^{TRAILER}: \S+@([0-9a-f]{{40}})\s*$", body, re.M)
    return m.group(1) if m else None


def merge(base, ours, theirs):
    """A page changed here (`ours`) and on the wiki (`theirs`) since `base`,
    merged line by line; None where they touch the same lines."""
    with tempfile.TemporaryDirectory() as d:
        paths = []
        for name, data in (("ours", ours), ("base", base), ("theirs", theirs)):
            p = os.path.join(d, name)
            with open(p, "wb") as f:
                f.write(data)
            paths.append(p)
        r = subprocess.run(["git", "merge-file", "-p", *paths], capture_output=True)
        return r.stdout if r.returncode == 0 else None


def plan(base, ours, theirs):
    """What to write to the wiki (path to bytes; None removes the page) and
    the pages that clash."""
    writes, clashes = {}, []
    for path in sorted(set(base) | set(ours)):
        b, o, t = base.get(path), ours.get(path), theirs.get(path)
        if o == b or t == o:
            # Not changed here, or the wiki has it already.
            continue
        if t == b:
            writes[path] = o
            continue
        # Changed here and on the wiki, differently: both kept if they can be.
        if o is not None and t is not None:
            m = merge(b or b"", o, t)
            if m is not None:
                writes[path] = m
                continue
        clashes.append(path)
    return writes, clashes


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--repo", required=True, help="the repository's checkout")
    p.add_argument("--wiki", required=True, help="the wiki's clone")
    p.add_argument("--dir", default="docs/wiki", help="the pages, in the repository")
    p.add_argument("--sha", required=True, help="the repository commit to publish")
    p.add_argument(
        "--imported-from",
        help="the wiki commit docs/wiki was taken from (for the first sync)",
    )
    p.add_argument(
        "--slug",
        default=os.environ.get("GITHUB_REPOSITORY", "repository"),
        help="owner/name, for the trailer",
    )
    a = p.parse_args(argv)
    # The trailer names a full commit, whatever --sha was given as.
    a.sha = git(a.repo, "rev-parse", "--verify", f"{a.sha}^{{commit}}").decode().strip()

    ours = tree(a.repo, a.sha, a.dir)
    theirs = tree(a.wiki, "HEAD")
    synced = last_sync(a.wiki)
    if synced:
        try:
            base = tree(a.repo, synced, a.dir)
        except subprocess.CalledProcessError:
            print(
                f"The wiki was last synced from {synced}, which this checkout "
                "doesn't have (fetch-depth: 0, or was main rewritten?)",
                file=sys.stderr,
            )
            return 1
    else:
        head = git(a.wiki, "rev-parse", "HEAD").decode().strip()
        if head != a.imported_from:
            print(
                f"The wiki has never been synced and is at {head}, not "
                f"{a.imported_from}, the commit {a.dir} was taken from: it "
                f"changed since. Bring those changes into {a.dir}, then set "
                "--imported-from to its commit.",
                file=sys.stderr,
            )
            return 1
        # docs/wiki was made from the wiki as it is: it wins page by page.
        base = theirs

    writes, clashes = plan(base, ours, theirs)
    if clashes:
        print(
            "Changed both on the wiki and here, in the same lines, so nothing "
            f"was written: {', '.join(clashes)}. Bring the wiki's version of "
            f"each into {a.dir} and merge again.",
            file=sys.stderr,
        )
        return 1

    for path, data in writes.items():
        full = os.path.join(a.wiki, path)
        if data is None:
            git(a.wiki, "rm", "-q", "--", path)
            continue
        os.makedirs(os.path.dirname(full) or a.wiki, exist_ok=True)
        with open(full, "wb") as f:
            f.write(data)
        git(a.wiki, "add", "--", path)

    staged = git(a.wiki, "diff", "--cached", "--name-only").decode().split()
    if not staged and synced:
        print("The wiki already has every page: nothing to publish")
        return 0
    subject = git(a.repo, "log", "-1", "--format=%s", a.sha).decode().strip()
    git(
        a.wiki,
        "-c", "user.name=github-actions[bot]",
        "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com",
        "commit", "-q", "--allow-empty",
        "-m", subject,
        "-m", f"{TRAILER}: {a.slug}@{a.sha}",
    )
    print(f"{len(staged)} page(s) to publish: {', '.join(staged) or 'none'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
