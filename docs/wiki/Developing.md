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

`IS_ALIVE_SHOTS_SCENES=1` as well takes the scene tour instead: each tool after a run, the Map with cards picked (Zones, the exploded view, 3D, Sites, the Light theme), Settings, each step of the Claude Handoff and the Site report window, each saved as `<dir>/<name>.png` once its run has finished. It's how a UI change is checked by eye at 1280 × 720 and 1120 × 720 (set `size=` in the settings file). The user manual has no screenshots: it's text only.

`IS_ALIVE_SHOTS_SELECT=<card id>` picks the card selected for the Map inspector shot, and `IS_ALIVE_SHOTS_CLIENTS=some` (or `all`) starts the Map with clients shown. On a pull request that touches `src/`, the **screenshots** workflow runs the tour in Dark and Light and attaches the images as an artifact.

## Measuring speed, memory and size

`IS_ALIVE_PERF=<file.json>` makes the app open the Contoso example map at 1280 × 720, time three phases of 15 seconds (the map left alone, a card picked with packets flowing, the 3D view spinning), write what it measured and close. A phase records each frame's gap on screen and its CPU time (eframe's, without the wait for vsync), the process's CPU use, and memory after each phase. Animate is on for the run, whatever Settings says. Nothing is saved: not the window size, not the site, not the results. GPU time and memory a dedicated GPU holds aren't seen.

On a Windows laptop, `scripts/perf.ps1` does it all. It runs a portable copy of the `.exe` from a temporary folder (so the user's settings stay as they are and an open is-alive can stay open), once with OpenGL and once with Direct3D 12. It then writes `perf-report.md` with the machine's CPU, GPU, screen and power, puts it on the clipboard, and opens its folder on the Desktop:

```powershell
powershell -ExecutionPolicy Bypass -File perf.ps1 -Exe .\is-alive.exe
powershell -ExecutionPolicy Bypass -File perf.ps1 -Exe .\is-alive.exe -Label battery
```

Keep the window in front and hands off while it runs: the map's packets stop when the window loses focus, and the report flags any phase where that happened or where the mouse or keys were used.

`scripts/size.sh` (Linux, or Git Bash on Windows; it needs `cargo-bloat` 0.12.1 and `rustup component add llvm-tools`) writes `size-report.md`: the shipped binary's size, code by crate, the biggest functions, the sections over 64 KB, and the files built in.

## Linux AppImage

```sh
packaging/linux/build-appimage.sh
# -> target/appimage/is-alive-x86_64.AppImage
```

Downloads `appimagetool` (release 1.9.1) and the AppImage runtime on first run and checks each against the SHA-256 pinned in the script, so the runtime that starts every released AppImage is the one reviewed (or set `APPIMAGETOOL=` and `APPIMAGE_RUNTIME=` to files of your own). The runtime is only published as `continuous`: when upstream rebuilds it the build stops with the new hash, to review and update. Only glibc is linked; GL, X11/Wayland and xkbcommon load from the host at runtime (`libxkbcommon-x11`, which stock Ubuntu lacks, is bundled), so the AppImage runs on any desktop distro with glibc ≥ the build host's (CI builds on Ubuntu 22.04 → glibc 2.35).

On Linux, ICMP uses unprivileged ping sockets (`SOCK_DGRAM`/`IPPROTO_ICMP`) — no root, no capabilities. That needs your group inside `net.ipv4.ping_group_range`, the default on Fedora, Ubuntu, Arch and most systemd distros. If it's not, the app falls back to the system `ping` command.

## Releases

CI builds the `.exe` and the `.AppImage` on every push/PR (Actions artifacts).

Versions: every pull request that changes what users get is its own release, and one pull request can carry several features or fixes. The version goes up once per pull request, by its size as a whole. A large change bumps the minor number (0.30.5 → 0.31.0): one that goes deep (reworks how a tool works underneath, or changes a file, bundle or settings format), one that goes broad (several tools or the shared window layout), or several large pieces together. Everything else bumps the patch (0.30.5 → 0.30.6). 1.0.0 is the owner's call.

To release, bump `version` in `Cargo.toml` (and `Cargo.lock`) in a PR and merge it to `main`: CI attests both binaries (`gh attestation verify <file> --repo hammerknife404/is-alive` checks one), tags `v<version>`, publishes a GitHub release with both binaries and checksums, and generates release notes from the PRs merged since the last release. Pushing a `v*` tag by hand does the same, as long as it matches `Cargo.toml`.

On other platforms (macOS etc.) ICMP falls back to the system `ping` command (dev/testing only).

## CI

| Workflow | When | What |
|---|---|---|
| **build** | every push and PR | fmt (Windows), clippy `-D warnings`, tests, the Windows `.exe` and Linux `.AppImage` as artifacts; on `main` with a new version, the release |
| **screenshots** | PRs touching `src/` | the screenshot tour in Dark and Light from the demo map, as an artifact |
| **weekly** | Mondays, by hand, and PRs touching `Cargo.*` | `cargo audit` (fails on a vulnerability), and a probe of every Speed Test preset server (`cargo test -- --ignored presets_answer`) |
| **measure** | by hand, and PRs touching `Cargo.*` or the measuring scripts | `scripts/size.sh` on Windows and Linux, in the job summary and as an artifact; on Windows, `scripts/perf.ps1` on the runner's software renderer, to check the script works (its timings mean nothing) |
| **wiki** | pushes to `main` and PRs touching `docs/wiki/` | publishes this wiki (below); a PR runs it all but the push |

## This wiki

These pages live in the repository, in [`docs/wiki/`](https://github.com/hammerknife404/is-alive/tree/main/docs/wiki), and change in the same pull request as what they describe. Merging to `main` publishes them: the **wiki** workflow merges what changed since the last sync into the wiki page by page, so an edit made here on GitHub is kept. Where both changed the same lines the run fails and writes nothing; bring the wiki's version into `docs/wiki/` and run it again. Each published commit names the repository commit it came from (`Synced-From:`).
