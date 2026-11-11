# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo build --release                      # -> target/release/is-alive(.exe)
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked                        # whole suite
cargo test --locked <name>                 # one test, e.g. the_manual_has_no_pictures
python3 skills/mapnetwork/scripts/test_engine_contract.py   # map engine contract (CI)
python3 scripts/test_wiki_sync.py          # wiki sync script tests
packaging/linux/build-appimage.sh          # -> target/appimage/is-alive-x86_64.AppImage
```

Rust edition 2024. Linux GUI needs a display; in a cloud sandbox use the scene tour (below) to see the window.

## Architecture

Native desktop app: Rust + egui/eframe, one binary, no installer. `src/main.rs` owns the `App`, the `Tab` enum/`TABS` table and the layout by window width; chrome (`chrome.rs`, `nav.rs`, `inspector.rs`, `widgets.rs`, `theme.rs`) is shared by every tab.

- **Tools** (`src/tools/*.rs`): one module per tab (ping, ports, trace, sweep, dns, advanced, export, notes, speed, calc, ...) holding its UI state and run logic. The network primitives they call live in `src/net/` (ICMP, TCP ports, DNS, TLS, MAC/OUI, names, subnet math, speed); keep UI out of `net/`.
- **Background work** goes through `src/job.rs`: a worker thread sends messages back over a channel (`Tx<T>`, which also requests a repaint and exposes the cancel flag); `pump` drains them on the UI thread. Long loops must poll `cancelled()` / use `Tx::sleep` so Stop stays responsive.
- **Configs in** (`src/unifi`, `src/sonicwall`, `src/meraki.rs`): offline parsers that turn vendor exports into sanitized JSON (`redact.rs`). Sanitizing happens locally before anything is saved or shared.
- **Map** (`src/map/`): builds the drawn network from the sweep, configs or a `/mapnetwork` map (`import.rs`), lays it out, computes findings and compares with the last visit (`compare.rs`, `runs.rs`). `src/mapnetwork.rs` is the contract with the bundled skill's map files.
- **Site / Bundle / Report**: `site.rs` keeps a visit's data on disk; `bundle.rs` + `tools/bundle.rs` zip the Site Bundle (manifest, kickoff prompt, bundled `skills/`); `report.rs` + `tools/report.rs` render the one-page HTML report; `results_ui.rs`, `runs_compare.rs` and `markdown_out.rs` handle Results and exports.
- **Docs are code-checked**: the manual (`docs/manual/MANUAL.md`) and Help (`help.rs`, `manual.rs`) are embedded and tested; the wiki pages in `docs/wiki/` sync to the GitHub wiki from CI (`scripts/wiki_sync.py`).
- `src/shots.rs` drives the headless screenshot tour used by `screenshots.yml`; `startup.rs`/`build.rs` handle renderer fallback (glow/wgpu) and resources.

## Releases: small and often

The owner wants point releases: each finished change ships on its own, not in a big batch.

- **One change, one PR, one release.** A PR that changes what users get (a fix, a feature, wording in the app) bumps the patch version in that same PR: `version` in `Cargo.toml` from 0.25.0 to 0.25.1, then `cargo check` so `Cargo.lock` follows (CI builds with `--locked`). Merging it to `main` publishes the release: `build.yml` tags `v<version>` and uploads both binaries with their checksums.
- **PRs go straight into `main`.** No `release-*` branch collecting several PRs.
- **Take the next free number when merging.** If another release landed first, merge `main` into the branch and bump to the next patch (0.25.2).
- **No bump** for what users don't get: docs, the wiki, CI, tests only.
- **x.y.0 only when the owner asks** for a feature release.
- **The PR title is the release note.** GitHub builds the notes from merged PR titles, so write them for users ("Sweep: several networks in one box").
- **Before asking to merge:** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`.
- **After merging:** check the `build` run on `main` passed and the release has `is-alive.exe`, `is-alive-x86_64.AppImage` and both `.sha256` files.
- **Wiki:** the app's Help (F1) opens the wiki (`is-alive.wiki`). Its pages live in `docs/wiki/`: add the release to `Release-notes.md` and update any page the release changes, in the same PR, as with the manual's text (`docs/manual/MANUAL.md`). Merging to `main` publishes them (`.github/workflows/wiki.yml`, `scripts/wiki_sync.py`): what changed since the last sync is merged into the wiki page by page, so an edit made on the wiki itself is kept. Where both changed the same lines, the run fails and writes nothing: bring the wiki's version into `docs/wiki/` and push again. Don't clone the wiki to edit it or hand out patches: a cloud session can't push it, and the workflow can.
- **The manual is text only**: no screenshots. Every PR that changes what a chapter of `docs/manual/MANUAL.md` describes updates that chapter. Say what's on screen in words, with the names the app shows and a table where it helps; an emoji or the app's own symbols (⋯ ▶ ■ ⟲ 📌 🔑) only where it makes a line quicker to find. `the_manual_has_no_pictures` (src/manual.rs) fails on an image link.
- **Checking the window by eye:** the scene tour (`IS_ALIVE_SHOTS=<dir> IS_ALIVE_SHOTS_SCENES=1`, src/shots.rs) saves a picture of each tool after a run, the Map with cards picked, Settings and Help, at the window size in the settings file (`size=1280x720`). A cloud sandbox has no ping, DNS or speed test: those scenes show their errors.

## UI: built for a field laptop first

Techs run this on site, on small laptops. Every UI change, and every step of the side-panel rework (mocks: https://claude.ai/artifact/6JjJnabWUj1QjwaVfc65zh), has to work there before it looks good on a big monitor.

- **Target 1280 × 720 points**: a 1080p laptop at 150% scaling. The app already opens at 1120 × 720 and allows 620 × 400; those have to keep working too.
- **Lay out by window width, not screen size.** Half a 4K monitor gets the same layout as a small laptop.
  - **1440 and up:** nav open (216), inspector docked (360), side panel docked (272).
  - **1100–1439:** nav open (216), the Map too. Inspector docked at 320 on tool pages; on the Map it floats over the canvas. Opening the Map's side panel closes the inspector.
  - **Under 1100:** nav folds to the icon rail (56). Inspector and side panels float over the content and close on Esc. Run-bar options wrap. Notes' Split falls back to Edit.
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
