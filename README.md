# is-alive

Portable network tester for MSP site visits, written in Rust. One `.exe` on Windows, one `.AppImage` on Linux: no installer, no admin rights, no telemetry.

> **Attending a demo?** Start with the plain-language **[follow-along guide](FOLLOW-ALONG.md)**.

## Download

Grab the latest build from [Releases](https://github.com/hammerknife404/is-alive/releases/latest): `is-alive.exe` (Windows) or `is-alive-x86_64.AppImage` (Linux, `chmod +x` then run).

## What it does

| Tab | Summary |
|---|---|
| **Ping** | ICMP (IPv4/IPv6) or TCP ping with a live latency graph — [details](https://github.com/hammerknife404/is-alive/wiki/Ping) |
| **Port Check** | Throttled TCP connect scan with presets (Windows/AD, Web, Firewall/VPN, Mail…) — [details](https://github.com/hammerknife404/is-alive/wiki/Port-Check) |
| **Traceroute** | 3 probes per hop, optional reverse DNS — [details](https://github.com/hammerknife404/is-alive/wiki/Traceroute) |
| **Subnet Sweep** | CIDR/range sweep with MAC, vendor, ARP-only devices and a JSON inventory — [details](https://github.com/hammerknife404/is-alive/wiki/Subnet-Sweep) |
| **DNS Lookup** | Any record type, or all at once, against several resolvers side by side — [details](https://github.com/hammerknife404/is-alive/wiki/DNS-Lookup) |
| **Advanced** | Mail and domain health: SPF, DMARC, DKIM, MX, SMTP/TLS, blacklists, DNSSEC… — [details](https://github.com/hammerknife404/is-alive/wiki/Domain-Health) |
| **Config Export** | Decodes UniFi, SonicWall and Meraki configs locally into sanitized JSON — [details](https://github.com/hammerknife404/is-alive/wiki/Config-Export) |
| **Notes** | Markdown tech notes with redaction of anything sensitive — [details](https://github.com/hammerknife404/is-alive/wiki/Notes) |
| **Map** | Network map built from sweeps and loaded configs — [details](https://github.com/hammerknife404/is-alive/wiki/Map) |
| **Speed Test** | Throughput, bufferbloat grade and VoIP MOS — [details](https://github.com/hammerknife404/is-alive/wiki/Speed-Test) |
| **Subnet Calc** | IPv4/IPv6 calculator, split and range-to-CIDR; works offline — [details](https://github.com/hammerknife404/is-alive/wiki/Subnet-Calc) |
| **Site Bundle…** | One zip of everything above for `/unifidump` and `/mapnetwork` — [details](https://github.com/hammerknife404/is-alive/wiki/Site-Bundle) |

Every result has **Copy**, and the **Results** tab exports to CSV, JSON, Markdown or HTML ([interface and export](https://github.com/hammerknife404/is-alive/wiki/Interface-and-Export)).

## Privacy

Nothing is sent unless you run a test. Configs and notes are sanitized on the machine before they are saved or shared, and the Results log lives only in memory. See [privacy and security](https://github.com/hammerknife404/is-alive/wiki/Privacy-and-Security).

## Build

```sh
cargo build --release                      # Windows .exe or Linux binary
packaging/linux/build-appimage.sh          # Linux AppImage
```

Cross-compiling, screenshots, CI and releases: [Building and Releasing](https://github.com/hammerknife404/is-alive/wiki/Building-and-Releasing).

## Documentation

Everything else is in the **[wiki](https://github.com/hammerknife404/is-alive/wiki)**: [Getting Started](https://github.com/hammerknife404/is-alive/wiki/Getting-Started), the [Site Visit Workflow](https://github.com/hammerknife404/is-alive/wiki/Site-Visit-Workflow), a page per tool, and [Troubleshooting](https://github.com/hammerknife404/is-alive/wiki/Troubleshooting).

The wiki's source is the [`wiki/`](wiki) folder; edits merged to `main` are published automatically.
