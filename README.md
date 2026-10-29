# is-alive

Native network tester written in Rust. Single portable `.exe` on Windows, single `.AppImage` on Linux — no installer, no admin rights.

| Tab | What it does |
|---|---|
| **Ping** | ICMP (IPv4/IPv6) or TCP-connect ping, continuous or N count, live latency graph, loss/min/avg/max |
| **Port Check** | Concurrent TCP connect scan — lists, ranges, presets (Common, Windows/AD, Web, Firewall/VPN, Mail, 1-1024) with service names |
| **Traceroute** | 3 probes per hop, optional reverse DNS |
| **Subnet Sweep** | Ping sweep of a CIDR or range (`10.0.0.0/24`, `10.0.0.1-254`, `10.0.0.10-10.0.1.20`) with hostnames |
| **DNS Lookup** | A/AAAA via the system resolver, PTR for IPs and for every resolved address |

Every result view has a **Copy** button for pasting into tickets.

### Scan speed

Port Check and Subnet Sweep have a **Speed** preset. Threads only cap how many probes are in flight; the **rate** (probes started per second, evenly spaced) is what stops a scan from bursting against hosts that answer fast.

| Preset | Rate | In flight | 1-1024 ports | /24 sweep |
|---|---|---|---|---|
| Gentle | 10/s | 4 | ~1m 43s | ~26 s |
| Normal (default) | 50/s | 16 | ~21 s | ~5 s |
| Fast | 500/s | 64 | ~2 s | <1 s |
| Unthrottled | none | 256 | tens of ms | tens of ms |

Any TCP connect or ICMP sweep is a scan by definition; throttling reduces noise and load but won't make it invisible to IDS/EDR. For sanctioned use, exclude the tool or the source host in the security stack.

ICMP goes through the Windows IP Helper API (`IcmpSendEcho2` / `Icmp6SendEcho2`), so it works as a normal user — no raw sockets.

## Build

```powershell
cargo build --release
# -> target\release\is-alive.exe
```

Cross-compile from Linux: `rustup target add x86_64-pc-windows-gnu`, install `mingw-w64`, then
`cargo build --release --target x86_64-pc-windows-gnu`.

### Linux AppImage

```sh
packaging/linux/build-appimage.sh
# -> target/appimage/is-alive-x86_64.AppImage
```

Downloads `appimagetool` on first run (or set `APPIMAGETOOL=/path/to/appimagetool`). Only glibc is linked; GL, X11/Wayland and xkbcommon load from the host at runtime, so the AppImage runs on any desktop distro with glibc ≥ the build host's (CI builds on Ubuntu 22.04 → glibc 2.35).

On Linux, ICMP uses unprivileged ping sockets (`SOCK_DGRAM`/`IPPROTO_ICMP`) — no root, no capabilities. That needs your group inside `net.ipv4.ping_group_range`, the default on Fedora, Ubuntu, Arch and most systemd distros. If it's not, the app falls back to the system `ping` command.

CI builds the `.exe` and the `.AppImage` on every push/PR (Actions artifacts).

To release, bump `version` in `Cargo.toml` (and `Cargo.lock`) in a PR and merge it to `main`: CI tags `v<version>`, publishes a GitHub release with both binaries and checksums, and generates release notes from the PRs merged since the last release. Pushing a `v*` tag by hand does the same, as long as it matches `Cargo.toml`.

On other platforms (macOS etc.) ICMP falls back to the system `ping` command (dev/testing only).
