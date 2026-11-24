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

## Code signing

Windows shows *Unknown Publisher* for an `.exe` without an Authenticode signature from a certificate it trusts. The build attestation above doesn't change that: Windows only names a publisher whose certificate comes from a code-signing CA, and since 2023 a CA keeps that key in a hardware security module, so a `.pfx` in a repository secret is no longer an option. The **build** workflow signs `is-alive.exe` for each release through [Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/overview) once it's set up, and ships it unsigned until then.

**What it takes:** a paid Azure subscription, and an organization in a [supported country](https://learn.microsoft.com/azure/artifact-signing/quickstart#prerequisites) (the US, Canada, the EU, the UK and a few others) or an individual developer in the US or Canada. Identity validation takes 1 to 30 business days. The Basic tier (5,000 signatures a month) is plenty; see [pricing](https://azure.microsoft.com/pricing/details/artifact-signing/). Windows shows the validated legal name as the publisher.

**Setting it up, once:**

1. **Azure:** register the `Microsoft.CodeSigning` resource provider on the subscription, then create an **Artifact Signing account** (Basic) in one of the [listed regions](https://learn.microsoft.com/azure/artifact-signing/quickstart#azure-regions-that-support-artifact-signing). Note its endpoint, such as `https://eus.codesigning.azure.net/`.
2. Give yourself the **Artifact Signing Identity Verifier** role on the account, create a **Public** identity validation (Organization, or Individual) and wait for **Completed**.
3. Create a **Public Trust** certificate profile on that validation.
4. **Entra ID:** create an app registration, and on it a federated credential for **GitHub Actions**: organization `hammerknife404`, repository `is-alive`, entity type **Environment**, environment `release-signing`. Give the app the **Artifact Signing Certificate Profile Signer** role on the signing account, and nothing else.
5. **GitHub, Settings → Environments:** create `release-signing`. Under **Deployment branches and tags** choose **Selected branches and tags** and add the branch `main` and the tag pattern `v*`. Add three environment secrets: `AZURE_CLIENT_ID` (the app registration), `AZURE_TENANT_ID` and `AZURE_SUBSCRIPTION_ID`.
6. **Settings → Secrets and variables → Actions → Variables** (repository variables): `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT` and `AZURE_SIGNING_PROFILE`. Setting `AZURE_SIGNING_ACCOUNT` turns signing on.

The next release is then signed. The **sign** job downloads the `.exe` the **windows** job built, signs and timestamps it, and checks the signature; the release's checksum and attestation are of the signed file. If signing fails, the release stops instead of shipping an unsigned `.exe`. To check a download: **Properties → Digital Signatures**, or `Get-AuthenticodeSignature is-alive.exe` in PowerShell.

**Why the environment:** the Azure credential accepts GitHub's sign-in only for the `release-signing` environment, and the environment's rule lets only `main` and `v*` tags use it. A workflow changed on any other branch can't sign. Signing runs in its own job, never on the runner where cargo runs the crates' build scripts.

**SmartScreen is separate.** A signed file can still get the blue *Windows protected your PC* box until it has built up download reputation, which happens on its own. After the first signed release, update "not code-signed yet" in `SECURITY.md` and the manual's *Why "Unknown Publisher"?* paragraph.

## CI

| Workflow | When | What |
|---|---|---|
| **build** | every push and PR | fmt (Windows), clippy `-D warnings`, tests, the Windows `.exe` and Linux `.AppImage` as artifacts; on `main` with a new version, the release (with the `.exe` signed once [code signing](#code-signing) is set up) |
| **screenshots** | PRs touching `src/` | the screenshot tour in Dark and Light from the demo map, as an artifact |
| **weekly** | Mondays, by hand, and PRs touching `Cargo.*` | `cargo audit` (fails on a vulnerability), and a probe of every Speed Test preset server (`cargo test -- --ignored presets_answer`) |
| **wiki** | pushes to `main` and PRs touching `docs/wiki/` | publishes this wiki (below); a PR runs it all but the push |

## This wiki

These pages live in the repository, in [`docs/wiki/`](https://github.com/hammerknife404/is-alive/tree/main/docs/wiki), and change in the same pull request as what they describe. Merging to `main` publishes them: the **wiki** workflow merges what changed since the last sync into the wiki page by page, so an edit made here on GitHub is kept. Where both changed the same lines the run fails and writes nothing; bring the wiki's version into `docs/wiki/` and run it again. Each published commit names the repository commit it came from (`Synced-From:`).
