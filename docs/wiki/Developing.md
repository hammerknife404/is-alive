## Build

```powershell
cargo build --release
# -> target\release\is-alive.exe
```

Cross-compile from Linux: `rustup target add x86_64-pc-windows-gnu`, install `mingw-w64`, then
`cargo build --release --target x86_64-pc-windows-gnu`.

No Rust on the host? Build, lint and test in a container:

```sh
podman run --rm --userns=keep-id -v $PWD:/src:Z -w /src docker.io/library/rust:latest \
  bash -c 'rustup component add clippy rustfmt && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test'
```

## Screenshots for UI work
`IS_ALIVE_SHOTS=<dir>` makes the app visit every tab, save `<dir>/NN-<tab>.png` of each, and close.
For the Map with data, make the fictional demo map and open it at the same time:

```sh
python3 scripts/demo-map.py demo.dc.html
IS_ALIVE_SHOTS=shots ./is-alive demo.dc.html
```

`IS_ALIVE_SHOTS_SCENES=1` as well takes the scene tour instead: each tool after a run, the Map with cards picked (Zones, Sites, the Light theme), Settings and the Site Bundle and Site report windows, each saved as `<dir>/<name>.png` once its run has finished. It's how a UI change is checked by eye at 1280 × 720 and 1120 × 720 (set `size=` in the settings file). The user manual has no screenshots: it's text only.

`IS_ALIVE_SHOTS_SELECT=<card id>` picks the card selected for the Map inspector shot, and `IS_ALIVE_SHOTS_CLIENTS=some` (or `all`) starts the Map with clients shown. On a pull request that touches `src/`, the **screenshots** workflow runs the tour in Dark and Light and attaches the images as an artifact.

## Linux AppImage

```sh
packaging/linux/build-appimage.sh
# -> target/appimage/is-alive-x86_64.AppImage
```

Downloads `appimagetool` on first run (or set `APPIMAGETOOL=/path/to/appimagetool`). Only glibc is linked; GL, X11/Wayland and xkbcommon load from the host at runtime (`libxkbcommon-x11`, which stock Ubuntu lacks, is bundled), so the AppImage runs on any desktop distro with glibc ≥ the build host's (CI builds on Ubuntu 22.04 → glibc 2.35).

On Linux, ICMP uses unprivileged ping sockets (`SOCK_DGRAM`/`IPPROTO_ICMP`) — no root, no capabilities. That needs your group inside `net.ipv4.ping_group_range`, the default on Fedora, Ubuntu, Arch and most systemd distros. If it's not, the app falls back to the system `ping` command.

## Releases

CI builds the `.exe` and the `.AppImage` on every push/PR (Actions artifacts).

Versions: a release with new features bumps the minor number (0.11.0); fixes and polish between feature releases bump the patch (0.11.1, 0.11.2, …).

To release, bump `version` in `Cargo.toml` (and `Cargo.lock`) in a PR and merge it to `main`: CI attests both binaries (`gh attestation verify <file> --repo hammerknife404/is-alive` checks one), tags `v<version>`, publishes a GitHub release with both binaries and checksums, and generates release notes from the PRs merged since the last release. Pushing a `v*` tag by hand does the same, as long as it matches `Cargo.toml`.

On other platforms (macOS etc.) ICMP falls back to the system `ping` command (dev/testing only).

## CI

| Workflow | When | What |
|---|---|---|
| **build** | every push and PR | fmt (Windows), clippy `-D warnings`, tests, the Windows `.exe` and Linux `.AppImage` as artifacts; on `main` with a new version, the release |
| **screenshots** | PRs touching `src/` | the screenshot tour in Dark and Light from the demo map, as an artifact |
| **weekly** | Mondays, by hand, and PRs touching `Cargo.*` | `cargo audit` (fails on a vulnerability), and a probe of every Speed Test preset server (`cargo test -- --ignored presets_answer`) |
| **wiki** | pushes to `main` and PRs touching `docs/wiki/` | publishes this wiki (below); a PR runs it all but the push |

## This wiki

These pages live in the repository, in [`docs/wiki/`](https://github.com/hammerknife404/is-alive/tree/main/docs/wiki), and change in the same pull request as what they describe. Merging to `main` publishes them: the **wiki** workflow merges what changed since the last sync into the wiki page by page, so an edit made here on GitHub is kept. Where both changed the same lines the run fails and writes nothing; bring the wiki's version into `docs/wiki/` and run it again. Each published commit names the repository commit it came from (`Synced-From:`).
