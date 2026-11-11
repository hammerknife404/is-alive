# is-alive

> **Attending a demo?** Start with the plain-language **[follow-along guide](FOLLOW-ALONG.md)**: download, open, and try it step by step. No technical background needed.

**📖 Documentation**, three ways in, each linking the others:

- **[Wiki](https://github.com/hammerknife404/is-alive/wiki)**: a short page per tab, and guides for the Site Bundle, settings, privacy and development.
- **[User manual](docs/manual/MANUAL.md)**: one document to read through or print. Every tab and control, worked examples, troubleshooting and a glossary. It's also built into the app: **Help** at the foot of the nav, or **F1** for the tab you're on.
- **[Follow-along guide](FOLLOW-ALONG.md)**: a 20-minute walk-through for people new to networking.

A native network tester for site visits, written in Rust. One portable `.exe` on Windows, one `.AppImage` on Linux: no installer, no admin rights. **[Download the latest release](https://github.com/hammerknife404/is-alive/releases/latest).**

| Tab | What it does |
|---|---|
| [Ping](https://github.com/hammerknife404/is-alive/wiki/Ping) | ICMP or TCP ping with a live latency graph, loss and min/avg/max |
| [Ports](https://github.com/hammerknife404/is-alive/wiki/Port-Check) | TCP port scan: lists, ranges and presets, with service names, and the TLS certificate on HTTPS and SSL-VPN ports (expiry, issuer, trust) |
| [Traceroute](https://github.com/hammerknife404/is-alive/wiki/Traceroute) | Every router to a host, three probes per hop |
| [Sweep](https://github.com/hammerknife404/is-alive/wiki/Subnet-Sweep) | Live hosts on one or more subnets and ranges at once, with names (DNS, else what each device calls itself over NetBIOS or mDNS), MACs, vendors and an Inventory for /mapnetwork |
| [Lookup](https://github.com/hammerknife404/is-alive/wiki/DNS-Lookup) | DNS: any record type from any resolver, or every resolver compared; WHOIS registration |
| [Domains](https://github.com/hammerknife404/is-alive/wiki/Advanced) | Mail and domain health: MX, SPF, DMARC, DKIM, SMTP, blacklists, DNSSEC and more |
| [Configs](https://github.com/hammerknife404/is-alive/wiki/Config-Export) | UniFi, SonicWall and Meraki configs to sanitized JSON, offline; /mapnetwork maps back in |
| [Map](https://github.com/hammerknife404/is-alive/wiki/Map) | The network drawn from the sweep, configs or a /mapnetwork map, with findings; compared with the last visit (new, changed, gone) |
| [Notes](https://github.com/hammerknife404/is-alive/wiki/Notes) | Markdown notes for the visit, with photos and redaction |
| [Speed](https://github.com/hammerknife404/is-alive/wiki/Speed-Test) | Throughput, latency, jitter and loss under load, bufferbloat grade and VoIP MOS |
| [Subnet Calc](https://github.com/hammerknife404/is-alive/wiki/Subnet-Calc) | IPv4 and IPv6 subnet calculator: details, address check, split, ranges to CIDR |
| [Results](https://github.com/hammerknife404/is-alive/wiki/Results) | Every finished run in a list, each one open beside it with Run again; kept with its site from one visit to the next, with Filter and Export to CSV, JSON, Markdown or HTML |

**Full scan** (top right) pings and port-checks the gateway, traces to the internet, tests DNS and sweeps the LAN in one go. **Report…** beside it puts the visit on one web page for the client or the ticket: the map, its findings, what changed since the last visit, every device and the notes, ready to print to PDF. **Settings** is at the foot of the nav on the left: the themes, the Miami backdrop and the network options. See [Settings and themes](https://github.com/hammerknife404/is-alive/wiki/Settings-and-themes).

## With Claude

**Site Bundle…** (in the site menu, top left) zips the visit's sanitized configs, sweep inventory and, if you choose, notes and results, with a manifest and a kickoff prompt for the `/unifidump` and `/mapnetwork` skills: one attach and one paste. With **Claude Skills** ticked the bundle carries both skills, so it works even where they aren't installed. Open the `.dc.html` map Claude makes back in Config Export and the Map redraws it, ready to be updated on the next visit. The Map's **More › Examples** opens two fictional sites drawn by `/mapnetwork`. See [Site Bundle and Claude](https://github.com/hammerknife404/is-alive/wiki/Site-Bundle-and-Claude).

## Privacy

Built for sites with regulated data. Nothing is sent unless you run something, apart from one DNS query to OpenDNS for the public IP and one request to GitHub for the latest release when it starts (both off in Settings; an update is only linked, never downloaded); there's no telemetry, update check or account. Configs are sanitized on this machine before they're saved or shared, notes are redacted, and copies stay out of Windows clipboard history. See [Privacy and security](https://github.com/hammerknife404/is-alive/wiki/Privacy-and-security).

## If the window doesn't open

- **"Windows protected your PC"** (a blue SmartScreen box): click **More info**, then **Run anyway**. Windows shows it for new programs that aren't from a large publisher.
- **Nothing happens on Windows:** right-click `is-alive.exe` → **Properties**, tick **Unblock** at the bottom (if it's there), then **OK**. Windows may block files downloaded from the internet; some antivirus software also holds back new programs, so check its quarantine.
- **No graphics driver, or a virtual machine without 3D:** is-alive draws its window with OpenGL. Where that can't start it starts itself again with Direct3D 12 on Windows, which works without a graphics card, or Vulkan on Linux, and keeps using that. If nothing works it says why in a message box.
- **To choose yourself** (also if the window opens but stays blank), start it once from a command prompt or a shortcut with `is-alive.exe --renderer=wgpu` (Direct3D or Vulkan; kept for later starts) or `--renderer=glow` (OpenGL, the default), or set `IS_ALIVE_RENDERER`.

Each start is written to `startup.log`: `%APPDATA%\is-alive\startup.log` on Windows (paste that into Explorer's address bar), `~/.config/is-alive/startup.log` on Linux. Attach it to an [issue](https://github.com/hammerknife404/is-alive/issues) if it still won't start.

## Build

```sh
cargo build --release
# -> target/release/is-alive(.exe)
packaging/linux/build-appimage.sh
# -> target/appimage/is-alive-x86_64.AppImage
```

Releases, CI, the screenshot tour and building in a container are in [Developing](https://github.com/hammerknife404/is-alive/wiki/Developing). Each version's highlights are in the [release notes](https://github.com/hammerknife404/is-alive/wiki/Release-notes). The [user manual](docs/manual/MANUAL.md) is text only and changes with each release that changes what it describes.

## License

[PolyForm Internal Use 1.0.0](LICENSE.md): any company may use and change is-alive for its own internal business operations, but not distribute it. Versions up to and including 0.23.0 were released under MIT, and that still applies to those versions.
