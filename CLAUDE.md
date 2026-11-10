# is-alive: notes for Claude

## Releases: small and often

The owner wants point releases: each finished change ships on its own, not in a big batch.

- **One change, one PR, one release.** A PR that changes what users get (a fix, a feature, wording in the app) bumps the patch version in that same PR: `version` in `Cargo.toml` from 0.25.0 to 0.25.1, then `cargo check` so `Cargo.lock` follows (CI builds with `--locked`). Merging it to `main` publishes the release: `build.yml` tags `v<version>` and uploads both binaries with their checksums.
- **PRs go straight into `main`.** No `release-*` branch collecting several PRs.
- **Take the next free number when merging.** If another release landed first, merge `main` into the branch and bump to the next patch (0.25.2).
- **No bump** for what users don't get: docs, the wiki, CI, tests only.
- **x.y.0 only when the owner asks** for a feature release. That's a "full release": the manual's screenshots are remade with `scripts/manual-shots.sh` (see the manual's last chapter). Patch releases never touch `docs/manual/img`.
- **The PR title is the release note.** GitHub builds the notes from merged PR titles, so write them for users ("Sweep: several networks in one box").
- **Before asking to merge:** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`.
- **After merging:** check the `build` run on `main` passed and the release has `is-alive.exe`, `is-alive-x86_64.AppImage` and both `.sha256` files.
- **Wiki:** the app's Help (F1) opens the wiki (`is-alive.wiki`). Add the release to its Release-notes page and update any page the release changes. A cloud session can clone the wiki but not push it (it isn't an attached repository): commit there and hand the owner the commit as a patch (`git format-patch -1`) with the commands to push it. The manual's text (`docs/manual/MANUAL.md`) is updated in the same PR as the change.
- **Manual screenshots** come from `scripts/manual-shots.sh` on the owner's machine (podman, a made-up LAN). A cloud sandbox has no ping, DNS or speed-test access: there, `--here --out <dir>` makes the offline pictures, and the network ones (ping, dns-lookup, advanced, speed-test, results) stay as they are.

## Bundled skills: keep them in step with the app

`skills/unifidump` and `skills/mapnetwork` ship inside every Site Bundle, and the Map opens their example maps. They're how Claude reads what is-alive writes, so they're part of the change, not docs to catch up on later.

- **Same PR.** A change to what a bundle carries or means updates `skills/` in that PR: a file in the zip, a manifest key, an `inventory.json` field or what one now holds (a MAC that can come from somewhere new), what `results.json` spans, or what is-alive reads back from a `/mapnetwork` map. Also update the matching `*_WHAT` text in `src/tools/bundle.rs` (the manifest's description of each file) and the inventory's `notes` in `src/tools/sweep.rs`.
- **Tests hold the line.** `the_bundled_mapnetwork_skill_knows_every_file` (src/bundle.rs) fails when a bundle file or manifest key isn't named in `skills/mapnetwork/SKILL.md`, and the inventory test in src/tools/sweep.rs fails when a host field isn't. Explain new things there; don't loosen the tests. `skills/mapnetwork/scripts/test_engine_contract.py` (CI) covers the map engine.
- **The source copy too.** `skills/` is a scrubbed copy that `scripts/sync-skills.py` makes from the owner's own skills, so an edit made only here is undone by the next sync. Hand the owner the same edits as a patch for the source skill (`git diff --relative=skills/<name> origin/main -- skills/<name>`), with the command to apply it.
- **Scrub.** Text added to `skills/` is public: no real site, client or person names, no full MACs (fictional names and documentation addresses only).
