# is-alive

> **Attending a demo?** Start with the plain-language **[follow-along guide](FOLLOW-ALONG.md)**: download, open, and try it step by step. No technical background needed.

**📖 Full documentation: the [is-alive wiki](https://github.com/hammerknife404/is-alive/wiki)**, with a page per tab and guides for the Site Bundle, settings, privacy and development.

A native network tester for site visits, written in Rust. One portable `.exe` on Windows, one `.AppImage` on Linux: no installer, no admin rights. **[Download the latest release](https://github.com/hammerknife404/is-alive/releases/latest).**

| Tab | What it does |
|---|---|
| [Ping](https://github.com/hammerknife404/is-alive/wiki/Ping) | ICMP or TCP ping with a live latency graph, loss and min/avg/max |
| [Port Check](https://github.com/hammerknife404/is-alive/wiki/Port-Check) | TCP port scan: lists, ranges and presets, with service names |
| [Traceroute](https://github.com/hammerknife404/is-alive/wiki/Traceroute) | Every router to a host, three probes per hop |
| [Subnet Sweep](https://github.com/hammerknife404/is-alive/wiki/Subnet-Sweep) | Live hosts on a subnet, with names, MACs, vendors and an Inventory for /mapnetwork |
| [DNS Lookup](https://github.com/hammerknife404/is-alive/wiki/DNS-Lookup) | Any record type from any resolver, or every resolver compared |
| [Advanced](https://github.com/hammerknife404/is-alive/wiki/Advanced) | Mail and domain health: MX, SPF, DMARC, DKIM, SMTP, blacklists, DNSSEC and more |
| [Config Export](https://github.com/hammerknife404/is-alive/wiki/Config-Export) | UniFi, SonicWall and Meraki configs to sanitized JSON, offline; /mapnetwork maps back in |
| [Map](https://github.com/hammerknife404/is-alive/wiki/Map) | The network drawn from the sweep, configs or a /mapnetwork map, with findings |
| [Notes](https://github.com/hammerknife404/is-alive/wiki/Notes) | Markdown notes for the visit, with redaction |
| [Speed Test](https://github.com/hammerknife404/is-alive/wiki/Speed-Test) | Throughput, latency, jitter and loss under load, bufferbloat grade and VoIP MOS |
| [Subnet Calc](https://github.com/hammerknife404/is-alive/wiki/Subnet-Calc) | IPv4 and IPv6 subnet calculator: details, address check, split, ranges to CIDR |
| [Results](https://github.com/hammerknife404/is-alive/wiki/Results) | Every finished run, with Filter and Export to CSV, JSON, Markdown or HTML |

**Full scan** (bottom right) pings and port-checks the gateway, traces to the internet, tests DNS and sweeps the LAN in one go. **Settings** opens from the app's name at the top left: nine themes, the Miami backdrop and keyboard shortcuts. See [Settings and themes](https://github.com/hammerknife404/is-alive/wiki/Settings-and-themes).

## With Claude

**Site Bundle…** (top right) zips the visit's sanitized configs, sweep inventory and, if you choose, notes and results, with a manifest and a kickoff prompt for the `/unifidump` and `/mapnetwork` skills: one attach and one paste. With **Claude Skills** ticked the bundle carries both skills, so it works even where they aren't installed. Open the `.dc.html` map Claude makes back in Config Export and the Map redraws it, ready to be updated on the next visit. The Map's **Examples** menu opens two fictional sites drawn by `/mapnetwork`. See [Site Bundle and Claude](https://github.com/hammerknife404/is-alive/wiki/Site-Bundle-and-Claude).

## Privacy

Built for sites with regulated data. Nothing is sent unless you run something, apart from one DNS query to OpenDNS for the public IP and one request to GitHub for the latest release when it starts (both off in Settings; an update is only linked, never downloaded); there's no telemetry, update check or account. Configs are sanitized on this machine before they're saved or shared, notes are redacted, and copies stay out of Windows clipboard history. See [Privacy and security](https://github.com/hammerknife404/is-alive/wiki/Privacy-and-security).

## Build

```sh
cargo build --release
# -> target/release/is-alive(.exe)
packaging/linux/build-appimage.sh
# -> target/appimage/is-alive-x86_64.AppImage
```

Releases, CI, the screenshot tour and building in a container are in [Developing](https://github.com/hammerknife404/is-alive/wiki/Developing). Each version's highlights are in the [release notes](https://github.com/hammerknife404/is-alive/wiki/Release-notes).

## License

[PolyForm Internal Use 1.0.0](LICENSE.md): any company may use and change is-alive for its own internal business operations, but not distribute it. Versions up to and including 0.23.0 were released under MIT, and that still applies to those versions.
