**is-alive** is a native network tester for site visits, written in Rust: one portable `.exe` on Windows, one `.AppImage` on Linux, no installer and no admin rights (the `.exe` carries its C runtime from 0.24.2, so a fresh Windows needs nothing else installed). It pings, scans, traces, sweeps, checks DNS and mail health, tests speed for VoIP, reads UniFi, SonicWall and Meraki configs offline, draws the network, keeps the visit's notes, and bundles it all for Claude's `/unifidump` and `/mapnetwork` skills.

**Download:** [latest release](https://github.com/hammerknife404/is-alive/releases/latest) · **Attending a demo?** [Follow-along guide](https://github.com/hammerknife404/is-alive/blob/main/FOLLOW-ALONG.md)

## Tabs

| Tab | What it does |
|---|---|
| [Ping](Ping) | ICMP or TCP ping with a live latency graph, loss and min/avg/max |
| [Port Check](Port-Check) | TCP port scan: lists, ranges and presets, with service names |
| [Traceroute](Traceroute) | Every router to a host, three probes per hop |
| [Subnet Sweep](Subnet-Sweep) | Live hosts on a subnet, with names, MACs, vendors and an Inventory for /mapnetwork |
| [DNS Lookup](DNS-Lookup) | Any record type from any resolver, or every resolver compared |
| [Advanced](Advanced) | Mail and domain health: MX, SPF, DMARC, DKIM, SMTP, blacklists, DNSSEC and more |
| [Config Export](Config-Export) | UniFi, SonicWall and Meraki configs to sanitized JSON, offline; /mapnetwork maps back in |
| [Map](Map) | The network drawn from the sweep, configs or a /mapnetwork map, with findings |
| [Notes](Notes) | Markdown notes for the visit, with redaction |
| [Speed Test](Speed-Test) | Throughput, latency, jitter and loss under load, bufferbloat grade and VoIP MOS |
| [Subnet Calc](Subnet-Calc) | IPv4 and IPv6 subnet calculator: details, address check, split, ranges to CIDR |
| [Domain join](Domain-join) | How this computer is joined (Active Directory, Entra ID, hybrid or workgroup) and to which domain, from `dsregcmd /status`; also in the status bar |
| [Results](Results) | Every finished run, with Filter and Export to CSV, JSON, Markdown or HTML |

Every tab is laid out the same way: the main input and **Start** in the run bar, the rest of the settings folded under **Options** (their values shown as small chips), a status row with **Copy** and **Save…**, and a short explanation before the first run. **Ctrl+1 … Ctrl+9** and **Ctrl+0** switch tabs. **Help** at the foot of the nav (or **F1**) opens this wiki.

When a newer release is out, the status bar shows **Update: vX.Y.Z**, a link to its release page (see [[Settings and themes|Settings-and-themes]]).

## Guides

- [[Site Bundle and Claude|Site-Bundle-and-Claude]]
- [[Settings and themes|Settings-and-themes]]
- [[Privacy and security|Privacy-and-security]]
- [[Developing|Developing]]
- [[Release notes|Release-notes]]
