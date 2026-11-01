**is-alive** is a portable network tester for MSP site visits. One `.exe` on Windows, one `.AppImage` on Linux: no installer, no admin rights, no telemetry.

New here? Start with **[Getting Started](Getting-Started)**, then the **[Site Visit Workflow](Site-Visit-Workflow)**.

## Tools

| Tab | Use it to |
|---|---|
| [Ping](Ping) | Check a host answers, watch latency and loss over time (ICMP or TCP) |
| [Port Check](Port-Check) | See which TCP services a host exposes |
| [Traceroute](Traceroute) | Find where along the path traffic stops or slows |
| [Subnet Sweep](Subnet-Sweep) | Inventory a subnet: who's alive, MAC, vendor, management ports |
| [DNS Lookup](DNS-Lookup) | Query any record, compare resolvers, spot split DNS |
| [Domain Health](Domain-Health) (**Advanced** tab) | Audit a domain's mail and DNS setup (SPF, DMARC, DKIM, SMTP, blacklists…) |
| [Config Export](Config-Export) | Turn UniFi, SonicWall and Meraki configs into sanitized JSON |
| [Map](Map) | Draw the network from sweeps and configs |
| [Notes](Notes) | Keep visit notes with secrets redacted |
| [Speed Test](Speed-Test) | Measure throughput, bufferbloat and VoIP call quality |
| [Subnet Calc](Subnet-Calc) | Work out blocks, masks, splits and ranges offline |
| [Site Bundle](Site-Bundle) | Package everything for `/unifidump` and `/mapnetwork` |

## Reference

- [Interface and Export](Interface-and-Export): settings, themes, shortcuts, Results log, CSV columns
- [Scan Speed](Scan-Speed): the Gentle / Normal / Fast / Unthrottled presets
- [Privacy and Security](Privacy-and-Security): what leaves the machine, and what never does
- [Troubleshooting](Troubleshooting)
- [Building and Releasing](Building-and-Releasing)
