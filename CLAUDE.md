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
- **Wiki:** the app's Help (F1) opens the wiki (`is-alive.wiki`). Add the release to its Release-notes page and update any page the release changes. The manual's text (`docs/manual/MANUAL.md`) is updated in the same PR as the change.
