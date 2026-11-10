#!/usr/bin/env python3
"""Tests for wiki_sync.py, on throwaway git repositories (CI: wiki.yml)."""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wiki_sync  # noqa: E402


def run(cwd, *args):
    return subprocess.run(
        ["git", "-C", cwd, *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def commit(repo, files, message="change"):
    """Write `files` (None removes one) and commit; the commit's sha."""
    for path, text in files.items():
        full = os.path.join(repo, path)
        if text is None:
            run(repo, "rm", "-q", "--", path)
            continue
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as f:
            f.write(text)
        run(repo, "add", "--", path)
    run(repo, "-c", "user.name=t", "-c", "user.email=t@example.com",
        "commit", "-q", "--allow-empty", "-m", message)
    return run(repo, "rev-parse", "HEAD")


def page(repo, path):
    full = os.path.join(repo, path)
    if not os.path.exists(full):
        return None
    with open(full) as f:
        return f.read()


class WikiSync(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = os.path.join(self.tmp.name, "repo")
        self.wiki = os.path.join(self.tmp.name, "wiki")
        for d in (self.repo, self.wiki):
            os.makedirs(d)
            run(d, "init", "-q")
        # The wiki as found, and docs/wiki taken from it.
        self.imported = commit(self.wiki, {"Home.md": "home\n", "Map.md": "a\nb\nc\n"})
        commit(self.repo, {"docs/wiki/Home.md": "home\n", "docs/wiki/Map.md": "a\nb\nc\n"})

    def tearDown(self):
        self.tmp.cleanup()

    def sync(self, sha, imported=None):
        return wiki_sync.main([
            "--repo", self.repo, "--wiki", self.wiki, "--sha", sha,
            "--imported-from", imported or self.imported, "--slug", "o/r",
        ])

    def test_the_first_sync_publishes_the_pages_and_marks_the_wiki(self):
        sha = commit(self.repo, {"docs/wiki/Map.md": "a\nB\nc\n", "docs/wiki/New.md": "new\n"})
        self.assertEqual(self.sync(sha), 0)
        self.assertEqual(page(self.wiki, "Map.md"), "a\nB\nc\n")
        self.assertEqual(page(self.wiki, "New.md"), "new\n")
        self.assertEqual(wiki_sync.last_sync(self.wiki), sha)

    def test_a_wiki_that_moved_before_its_first_sync_is_left_alone(self):
        commit(self.wiki, {"Home.md": "edited on the wiki\n"})
        head = run(self.wiki, "rev-parse", "HEAD")
        sha = commit(self.repo, {"docs/wiki/Map.md": "x\n"})
        self.assertEqual(self.sync(sha), 1)
        self.assertEqual(run(self.wiki, "rev-parse", "HEAD"), head)

    def test_edits_on_the_wiki_are_kept(self):
        self.assertEqual(self.sync(commit(self.repo, {"docs/wiki/Map.md": "a\nB\nc\n"})), 0)
        # On the wiki: a page of its own, and Home edited.
        commit(self.wiki, {"Only-there.md": "mine\n", "Home.md": "home, edited\n"})
        sha = commit(self.repo, {"docs/wiki/Map.md": "a\nB\nC\n"})
        self.assertEqual(self.sync(sha), 0)
        self.assertEqual(page(self.wiki, "Map.md"), "a\nB\nC\n")
        self.assertEqual(page(self.wiki, "Home.md"), "home, edited\n")
        self.assertEqual(page(self.wiki, "Only-there.md"), "mine\n")

    def test_a_page_changed_in_both_places_is_merged_line_by_line(self):
        self.assertEqual(self.sync(commit(self.repo, {"docs/wiki/New.md": "n\n"})), 0)
        commit(self.wiki, {"Map.md": "a\nb\nc, on the wiki\n"})
        sha = commit(self.repo, {"docs/wiki/Map.md": "a here\nb\nc\n"})
        self.assertEqual(self.sync(sha), 0)
        self.assertEqual(page(self.wiki, "Map.md"), "a here\nb\nc, on the wiki\n")

    def test_a_clash_writes_nothing(self):
        self.assertEqual(self.sync(commit(self.repo, {"docs/wiki/New.md": "n\n"})), 0)
        commit(self.wiki, {"Map.md": "a\nwiki\nc\n"})
        head = run(self.wiki, "rev-parse", "HEAD")
        sha = commit(self.repo, {"docs/wiki/Map.md": "a\nhere\nc\n", "docs/wiki/Home.md": "h2\n"})
        self.assertEqual(self.sync(sha), 1)
        self.assertEqual(run(self.wiki, "rev-parse", "HEAD"), head)
        self.assertEqual(page(self.wiki, "Home.md"), "home\n")

    def test_a_page_removed_here_leaves_the_wiki(self):
        self.assertEqual(self.sync(commit(self.repo, {"docs/wiki/New.md": "n\n"})), 0)
        sha = commit(self.repo, {"docs/wiki/New.md": None})
        self.assertEqual(self.sync(sha), 0)
        self.assertIsNone(page(self.wiki, "New.md"))

    def test_the_trailer_names_the_full_commit(self):
        sha = commit(self.repo, {"docs/wiki/New.md": "n\n"})
        self.assertEqual(self.sync("HEAD"), 0)
        self.assertEqual(wiki_sync.last_sync(self.wiki), sha)

    def test_nothing_new_makes_no_commit(self):
        sha = commit(self.repo, {"docs/wiki/New.md": "n\n"})
        self.assertEqual(self.sync(sha), 0)
        head = run(self.wiki, "rev-parse", "HEAD")
        self.assertEqual(self.sync(commit(self.repo, {"README.md": "x\n"})), 0)
        self.assertEqual(run(self.wiki, "rev-parse", "HEAD"), head)


if __name__ == "__main__":
    unittest.main()
