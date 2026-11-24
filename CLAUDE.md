# is-alive: notes for Claude

The rules here apply to every change. Reference specs live in `docs/dev/`; they aren't loaded on their own, so read the one named before working in its area.

## Releases

One PR, one release, straight into `main` (no `release-*` branches). Merging publishes it: `build.yml` tags `v<version>` and uploads both binaries with checksums. Build, CI and wiki mechanics: `docs/wiki/Developing.md`.

- **A PR can carry several features or fixes.** Each one is finished, tested and covered in the manual, wiki and skills before the PR asks to merge; don't merge half of one to ship the rest.
- **Bump the version once per PR** when users get something different (a fix, a feature, wording in the app). Pick the step by the size of the PR as a whole:
  - **Minor (0.X.Y → 0.X+1.0)** when the change is large: deep (reworks how a tool works underneath, or changes a file, bundle or settings format), broad (user-visible changes across several tools or the shared shell: nav, inspector, layout), or several large pieces together.
  - **Patch (0.X.Y → 0.X.Y+1)** for everything else.
  - On a judgment call, say in the PR which you picked and why, so the owner can overrule it before merging. **1.0.0 only when the owner asks.**
- **Where the number goes:** `version` in `Cargo.toml`, then `cargo check` so `Cargo.lock` follows (CI builds with `--locked`). The same number goes in the manual's intro paragraph (`**is-alive X.Y.Z**`, the only place `docs/manual/MANUAL.md` may name it) and step 4 of `FOLLOW-ALONG.md`. `the_manual_and_the_guide_name_this_version` (src/manual.rs) checks all three.
- **No bump** for docs, the wiki, CI or tests only, and label the PR `internal` so its title stays out of the next release's notes (`.github/release.yml`).
- **Another release landed first?** Merge `main` into the branch and take the next free number at the same step.
- **The PR title is the release note** (GitHub builds the notes from merged PR titles), so write it for users and name every feature in it: "Sweep: several networks in one box; Ping: TCP mode". `Release-notes.md` in the wiki gets a line for each.
- **Before asking to merge:** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`.
- **After merging:** the `build` run on `main` passed, and the release has `is-alive.exe`, `is-alive-x86_64.AppImage` and both `.sha256` files.

## Docs that change with the code

- **Manual** (`docs/manual/MANUAL.md`, built into the app): a PR that changes what a chapter describes updates that chapter. Text only, no screenshots (`the_manual_has_no_pictures`): say what's on screen in words, with the names the app shows and a table where it helps; an emoji or the app's own symbols (⋯ ▶ ■ ⟲ 📌 🔑) only where it makes a line quicker to find.
- **Wiki** (`docs/wiki/`, opened by Help/F1): add the release to `Release-notes.md` and update any page it changes, in the same PR. The `wiki` workflow publishes on merge and keeps edits made on the wiki itself. Don't clone the wiki or hand out patches for it: a cloud session can't push it. If the sync reports a conflict, bring the wiki's version into `docs/wiki/` and push again.

## UI: field laptop first

Techs run this on site, on small laptops. It has to work there before it looks good on a big monitor.

- **Target 1280 × 720 points** (1080p at 150%). The default 1120 × 720 and the minimum 620 × 400 keep working too.
- **Lay out by window width, never screen size.** Height is the tighter limit.
- **Click targets at least 28 points tall**, for a trackpad on a tailgate.
- Breakpoints, docked vs floating panels, the 130-point chrome budget, the Map's zoom, the column drop order and the side-panel rework mocks: **`docs/dev/ui-layout.md`**. Read it before touching layout.
- **Check every UI change at 1280 × 720 and 1120 × 720** with the scene tour (`IS_ALIVE_SHOTS=<dir> IS_ALIVE_SHOTS_SCENES=1`, window size from `size=` in the settings file), and say in the PR that you did. A cloud sandbox has no ping, DNS or speed test, so those scenes show errors; that's expected.

## Bundled skills ship with the app

`skills/unifidump` and `skills/mapnetwork` go in every Site Bundle and are how Claude reads what is-alive writes. They're part of the change, not docs to catch up on later.

- **Same PR.** Changing what a bundle carries or means updates `skills/`, the `*_WHAT` text in `src/tools/bundle.rs` and the inventory's `notes` in `src/tools/sweep.rs`.
- **Never loosen the tests** that guard this; explain the new thing in the skill instead. If a sync brought older text back, restore the bundled text from git and point it out.
- **The owner keeps the source.** `skills/` is a scrubbed copy (`scripts/sync-skills.py`); don't hand the owner patches for their copy unless asked.
- **Public text.** Fictional names and documentation addresses only, and no full MAC in any form.
- What counts as a bundle change, which test checks what, and every MAC form the scrub catches: **`docs/dev/bundled-skills.md`**.
