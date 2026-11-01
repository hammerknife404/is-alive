# is-alive

Portable network tester for MSP site visits, written in Rust. One `.exe` on Windows, one `.AppImage` on Linux: no installer, no admin rights, no telemetry.

> **Attending a demo?** Start with the plain-language **[follow-along guide](FOLLOW-ALONG.md)**.

## Download

Grab the latest build from [Releases](https://github.com/hammerknife404/is-alive/releases/latest): `is-alive.exe` (Windows) or `is-alive-x86_64.AppImage` (Linux, `chmod +x` then run).

## What it does

| Tab | Summary |
|---|---|
| **Ping** | ICMP (IPv4/IPv6) or TCP ping with a live latency graph |
| **Port Check** | Throttled TCP connect scan with presets (Windows/AD, Web, Firewall/VPN, Mail…) |
| **Traceroute** | 3 probes per hop, optional reverse DNS |
| **Subnet Sweep** | CIDR/range sweep with MAC, vendor, ARP-only devices and a JSON inventory — [details](docs/scanning.md) |
| **DNS Lookup** | Any record type, or all at once, against several resolvers side by side |
| **Advanced** | Mail and domain health: SPF, DMARC, DKIM, MX, SMTP/TLS, blacklists, DNSSEC… — [details](docs/domain-health.md) |
| **Config Export** | Decodes UniFi, SonicWall and Meraki configs locally into sanitized JSON — [details](docs/config-export.md) |
| **Notes** | Markdown tech notes with redaction of anything sensitive — [details](docs/notes.md) |
| **Map** | Network map built from sweeps and loaded configs — [details](docs/map.md) |
| **Speed Test** | Throughput, bufferbloat grade and VoIP MOS — [details](docs/speed-test.md) |
| **Subnet Calc** | IPv4/IPv6 calculator, split and range-to-CIDR; works offline |
| **Site Bundle…** | One zip of everything above for `/unifidump` and `/mapnetwork` — [details](docs/site-bundle.md) |

Every result has **Copy**, and the **Results** tab exports to CSV, JSON, Markdown or HTML ([interface and export](docs/interface.md)).

## Privacy

Nothing is sent unless you run a test. Configs and notes are sanitized on the machine before they are saved or shared, and the Results log lives only in memory. See [privacy and security](docs/privacy.md).

## Build

```sh
cargo build --release                      # Windows .exe or Linux binary
packaging/linux/build-appimage.sh          # Linux AppImage
```

Cross-compiling, screenshots, CI and releases: [docs/building.md](docs/building.md).

## Documentation

- [Interface, Results and export](docs/interface.md)
- [Scanning: Subnet Sweep and scan speed](docs/scanning.md)
- [Advanced (domain health)](docs/domain-health.md)
- [Speed Test](docs/speed-test.md)
- [Config Export: UniFi, SonicWall, Meraki](docs/config-export.md)
- [Map](docs/map.md)
- [Notes](docs/notes.md)
- [Site Bundle](docs/site-bundle.md)
- [Privacy and security](docs/privacy.md)
- [Building and releasing](docs/building.md)
