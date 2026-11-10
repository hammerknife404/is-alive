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

## UI: built for a field laptop first

Techs run this on site, on small laptops. Every UI change, and every step of the side-panel rework (mocks: https://claude.ai/artifact/6JjJnabWUj1QjwaVfc65zh), has to work there before it looks good on a big monitor.

- **Target 1280 × 720 points**: a 1080p laptop at 150% scaling. The app already opens at 1120 × 720 and allows 620 × 400; those have to keep working too.
- **Lay out by window width, not screen size.** Half a 4K monitor gets the same layout as a small laptop.
  - **1440 and up:** nav open (216), inspector docked (360), side panel docked (272).
  - **1100–1439:** nav folds to the icon rail (56). Inspector docked at 320 on tool pages; on the Map it floats over the canvas. Opening the Map's side panel closes the inspector.
  - **Under 1100:** inspector and side panels float over the content and close on Esc. Run-bar options wrap. Notes' Split falls back to Edit.
- **Height is the tighter limit.** At 720, the title bar, status bar and run bar together stay under 130 points, so results get most of the window.
- **The Map keeps its zoom** when a panel opens over it. Panels float over the canvas; they don't shrink it.
- **Columns drop in a fixed order** as the window narrows (Vendor first, then MAC). Anything a row loses still shows in the inspector.
- **Click targets stay at least 28 points tall**, for a trackpad on a tailgate.
- **Check every UI change at 1280 × 720 and at 1120 × 720** before asking to merge, and say in the PR that you did.

## Bundled skills: keep them in step with the app

`skills/unifidump` and `skills/mapnetwork` ship inside every Site Bundle, and the Map opens their example maps. They're how Claude reads what is-alive writes, so they're part of the change, not docs to catch up on later.

- **Same PR.** A change to what a bundle carries or means updates `skills/` in that PR: a file in the zip, a manifest key, an `inventory.json` field or what one now holds (a MAC that can come from somewhere new), what `results.json` spans, or what is-alive reads back from a `/mapnetwork` map. Also update the matching `*_WHAT` text in `src/tools/bundle.rs` (the manifest's description of each file) and the inventory's `notes` in `src/tools/sweep.rs`.
- **Tests hold the line.** `the_bundled_mapnetwork_skill_knows_every_file` (src/bundle.rs) fails when a bundle file or manifest key isn't named in `skills/mapnetwork/SKILL.md`, and the inventory test in src/tools/sweep.rs fails when a host field isn't. Explain new things there; don't loosen the tests. `skills/mapnetwork/scripts/test_engine_contract.py` (CI) covers the map engine.
- **The bundled copies are Claude's to keep current; the owner keeps the source.** `skills/` is a scrubbed copy that `scripts/sync-skills.py` makes from the owner's own skills. The owner patches their source copy themselves; don't hand them patches for it unless they ask. If a sync brings older text back into `skills/`, the tests above fail: restore the bundled text from git (and point it out), don't loosen the tests.
- **Scrub.** Text added to `skills/` is public: no real site, client or person names, no full MACs (fictional names and documentation addresses only).
