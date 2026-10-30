# is-alive

> **Attending a demo?** Start with the plain-language **[follow-along guide](FOLLOW-ALONG.md)**: download, open, and try it step by step. No technical background needed.

Native network tester written in Rust. Single portable `.exe` on Windows, single `.AppImage` on Linux — no installer, no admin rights.

| Tab | What it does |
|---|---|
| **Ping** | ICMP (IPv4/IPv6) or TCP-connect ping, continuous or N count, live latency graph, loss/min/avg/max |
| **Port Check** | Concurrent TCP connect scan — lists, ranges, presets (Common, Windows/AD, Web, Firewall/VPN, Mail, 1-1024) with service names. TCP only: UDP services such as IKE (500/4500) can't be tested |
| **Traceroute** | 3 probes per hop, optional reverse DNS |
| **Subnet Sweep** | Ping sweep of a CIDR or range (`10.0.0.0/24`, `10.0.0.1-254`, `10.0.0.10-10.0.1.20`) with hostnames, MAC and vendor, optional management-port check, and an **Inventory** JSON for /mapnetwork (see below) |
| **DNS Lookup** | Any record type, or **All** at once (A/AAAA, MX, TXT, CNAME, NS, SOA, SRV, CAA, PTR, DS, DNSKEY) via the system resolver, Cloudflare, Google, Quad9 or the domain's authoritative server, or **Compare all** side by side to spot split DNS and stale caches |
| **Advanced** | Mail and domain DNS health for a domain — validates, not just detects (see below) |
| **Config Export** | Decodes a UniFi backup (`.unf` / `.unifi`) or SonicWall settings export (`.exp`) on this machine and saves sanitized JSON for the unifidump / mapnetwork skills (see below) |

Every result view has a **Copy** button for pasting into tickets. The **Results** tab also has a **Filter** box (tool name or output text) and **Export** to CSV, JSON, Markdown or HTML, for the whole log or a single run; Copy, Save and Export all use the filtered set.

CSV export uses each tool's own columns, one row per result, with the run's start time first, so repeated runs stack and filter cleanly in Excel:

| Tool | Columns |
|---|---|
| Ping | target, mode, port, seq, from, status, rtt_ms, ttl, bytes |
| Port Check | host, ip, port, state, service, rtt_ms, error (closed/filtered included) |
| Traceroute | host, ip, hop, address, name, rtt1_ms, rtt2_ms, rtt3_ms, note |
| Subnet Sweep | range, ip, rtt_ms, ttl, mac, vendor, name, open_ports (alive hosts) |
| DNS Lookup | lookup, source, type, value (Compare notes as type `note`) |
| Advanced | domain, resolver, section, check, check_status, kind (finding/record), status, text |
| Host info | field, value |

Exporting the whole log gives one CSV: a `tool` column plus every tool's columns (shared ones like `ip` and `rtt_ms` line up), so filter on `tool` in Excel. Runs that failed before producing results carry their text in an `output` column.

### Advanced (domain health)

Enter a domain (`example.com`, `user@example.com` or a URL all work) and press Enter. The checks run in parallel against a public resolver queried directly (default `1.1.1.1`, any IP accepted) rather than the system one, so you see what the internet sees, not hosts-file or split-DNS answers. Each check reports PASS / WARN / FAIL / INFO with the reason and the raw records; checks with warnings or failures open automatically. **Stop** ends a run immediately, and results go to Copy, Save and the Results log like every other tab.

| Check | Validates |
|---|---|
| **MX** | missing, IP literal, CNAME target, host doesn't resolve, null MX (RFC 7505); detects Microsoft 365 / Google |
| **SPF** | missing or multiple records; DNS lookups against the 10 limit, counting every evaluation (an include reached twice counts twice); include loops; void lookups (`a`/`mx`/`exists` targets with no records, limit 2); `+all` / `?all` / no `all`, following `redirect=` to the record that holds it; `ptr`; includes with no SPF |
| **DMARC** | missing or duplicate, `p=none`, `sp=none`, no `rua=`, `pct<100`, external report domains not authorized; falls back to the organizational domain |
| **DKIM** | probes `selector1/2` (M365), `google`, `k1-3` (Mailchimp), `s1/s2` (SendGrid), `default`, … plus any extra selectors you enter; key size (<1024 fail, <2048 warn), naming the provider when the key sits behind its CNAME; revoked keys, warning when no active key is found; M365-style CNAMEs with no key behind them; a selector that fails to answer is reported without losing the others |
| **Nameservers** | fewer than 2 NS; each NS queried directly (no answer, lame/non-authoritative, serial drift); registrar/parent delegation vs the zone's NS set |
| **SMTP (port 25)** | connects to the first 3 MX hosts: 220 greeting, STARTTLS, TLS version, certificate validity and expiry, and an open-relay test (stops at RCPT, never sends mail). If every MX times out, reports that this network probably blocks outbound port 25 |
| **Blacklists** | self-hosted MX IPs, single `ip4:` SPF addresses and this network's WAN IP against Spamhaus ZEN, Barracuda, SpamCop, PSBL, Mailspike, UCEPROTECT L1, NiX Spam and DroneBL; the domain against Spamhaus DBL, URIBL and SURBL. Spamhaus PBL (end-user ranges) is INFO unless SPF authorizes the IP. Lists that refuse public resolvers are retried via this machine's DNS server |
| **MTA-STS** | TXT plus HTTPS policy: mode, max_age, every MX covered; says plainly when `mta-sts.<domain>` doesn't resolve |
| **TLS-RPT** | present, valid `rua=` |
| **BIMI** | HTTPS SVG logo (query string allowed), VMC/CMC, DMARC at quarantine/reject with pct=100 |
| **DNSSEC** | DS/DNSKEY, signed but no DS, bogus (SERVFAIL that clears with CD), resolver-validated (AD), deprecated algorithms / SHA-1 DS |
| **CAA** | allowed CAs (inherited from parent domains), critical unknown tags |
| **MX reverse DNS** | PTR on every MX address, forward-confirmed. WARN for self-hosted MX (it usually sends mail too); INFO for hosted services (M365, Google, Cloudflare, Mimecast, Proofpoint, Barracuda), whose inbound relays don't send your mail |
| **Autodiscover** | CNAME to `autodiscover.outlook.com` when mail is on Microsoft 365 |
| **Domain expiry** | RDAP via the IANA bootstrap (fetched once per session): expiry date, registrar, hold / pending-delete status |

DKIM keys can't be listed, only probed. If none of the common selectors match, take the `s=` value from a received message's `DKIM-Signature` header and add it under **Extra DKIM selectors**.

MTA-STS and RDAP use HTTPS with the OS certificate store (works behind TLS-inspecting firewalls) and honor `HTTPS_PROXY` / the Windows system proxy.

### Subnet Sweep: MAC, vendor and inventory

Each live host's **MAC** comes from this machine's ARP table (Windows `GetIpNetTable2`, Linux `/proc/net/arp`; no admin rights), so only hosts on your own segment have one. **Vendor** is the IEEE registry owner of the MAC prefix (MA-L, MA-M and MA-S, bundled offline; rebuild with `python3 scripts/update-oui.py`). A locally administered MAC shows as *private/virtual*: a phone or laptop's per-network private address, or a VM/container NIC.

**Check mgmt ports** adds a TCP connect after the sweep to each live host on 22, 23, 80, 443, 445, 541 (FortiGate), 554 (RTSP), 3389, 4433 (SonicWall), 5000, 8080/8443 (UniFi), 8291 (MikroTik) and 9100 (printers), at the same speed limit. Open management ports are the evidence for telling switches, APs and firewalls from clients. It's off by default: it is a port scan.

**Inventory** saves the run as JSON for `/mapnetwork`'s scan input: this machine's LAN, gateway and WAN, the ports checked, and per host `ip`, `mac`, `vendor`, `private_mac`, `name`, `rtt_ms`, `ttl`, `open_ports`, `is_gateway` and `is_this_machine`. Facts only, no role guesses; the MACs are what join it to `/unifidump`'s `discovered_devices` and uplink MACs.

### Config Export: UniFi

**Open file…** takes a `.unf` (site export or classic autobackup), a `.unifi` (UniFi OS console backup), a `.zip` holding one, or a bare `db.gz`; the format is detected from the content, not the name. Everything runs locally: nothing is uploaded or sent anywhere.

| File | Decrypt | Inside |
|---|---|---|
| `.unf` | AES-128-CBC, UniFi's static key | zip → `db.gz` |
| `.unifi` | AES-256-CBC, UniFi's static key, IV in the first 16 bytes | gzip → tar → `backup/network/db.gz` |

**Save JSON…** writes `<name>.unifidump.json`: the mongoexport-style array (`{"__cmd":"select","collection":…}` markers) that the unifidump skill's `extract.py` reads directly, so hand that file to `/unifidump` instead of the backup.

- **Kept:** `site`, `device`, `networkconf`, `portconf`, `wlanconf`, and `user` (clients) only when you tick **Include UniFi clients** (off by default). Client hostnames are often people's names; unifidump uses them only for counts and to spot unadopted gear plugged in as a client.
- **Left behind:** every other collection: admins and their password hashes, settings, events, alarms, stats, rogue APs, and so on. The console's own config, TLS keys and Postgres database in a `.unifi` are never read.
- **Removed from what's kept:** any field starting `x_` (UniFi's prefix for stored secrets: WLAN passphrases, device auth keys, WAN/VPN passwords) or named like a password, passphrase, secret, PSK, private key, token, certificate or hash, at any depth.

The summary lists the layers peeled, the collections kept and left behind with counts, and every field removed, so you can check what goes out before sharing it. Names, MACs, IPs, VLANs and topology stay: that is what unifidump maps.

### Config Export: SonicWall

The same **Open file…** takes a SonicWall settings export (`.exp`, Gen 6 or 7), or one already decoded to text. The file is only base64 and URL-encoding; it's decoded here and turned into JSON for `/mapnetwork`: interfaces (zone, physical/VLAN with tag and parent, IP/mask, gateway, secondary subnet, port MAC, which management protocols are on, comment), zones, address objects and groups, FQDN objects, service objects and groups, access rules (zones, source, destination, service, action, enabled, comment) and NAT rules.

The output is built only from those named fields, so admin and user accounts, passwords, VPN pre-shared keys, RADIUS/LDAP secrets, SNMP communities, certificates and PPPoE logins never leave the file; the summary says how many settings were copied and how many left behind. Comments are kept because they say what a rule or port is for, so check them if your team ever wrote credentials into one. Interface MACs are what `/mapnetwork` matches against `/unifidump`'s unresolved uplink MACs to name the firewall.

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

Downloads `appimagetool` on first run (or set `APPIMAGETOOL=/path/to/appimagetool`). Only glibc is linked; GL, X11/Wayland and xkbcommon load from the host at runtime (`libxkbcommon-x11`, which stock Ubuntu lacks, is bundled), so the AppImage runs on any desktop distro with glibc ≥ the build host's (CI builds on Ubuntu 22.04 → glibc 2.35).

On Linux, ICMP uses unprivileged ping sockets (`SOCK_DGRAM`/`IPPROTO_ICMP`) — no root, no capabilities. That needs your group inside `net.ipv4.ping_group_range`, the default on Fedora, Ubuntu, Arch and most systemd distros. If it's not, the app falls back to the system `ping` command.

CI builds the `.exe` and the `.AppImage` on every push/PR (Actions artifacts).

To release, bump `version` in `Cargo.toml` (and `Cargo.lock`) in a PR and merge it to `main`: CI tags `v<version>`, publishes a GitHub release with both binaries and checksums, and generates release notes from the PRs merged since the last release. Pushing a `v*` tag by hand does the same, as long as it matches `Cargo.toml`.

On other platforms (macOS etc.) ICMP falls back to the system `ping` command (dev/testing only).
