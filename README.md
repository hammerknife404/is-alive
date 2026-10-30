# is-alive

> **Attending a demo?** Start with the plain-language **[follow-along guide](FOLLOW-ALONG.md)**: download, open, and try it step by step. No technical background needed.

Native network tester written in Rust. Single portable `.exe` on Windows, single `.AppImage` on Linux — no installer, no admin rights.

| Tab | What it does |
|---|---|
| **Ping** | ICMP (IPv4/IPv6) or TCP-connect ping, continuous or N count, live latency graph, loss/min/avg/max |
| **Port Check** | Concurrent TCP connect scan — lists, ranges, presets (Common, Windows/AD, Web, Firewall/VPN, Mail, 1-1024) with service names. TCP only: UDP services such as IKE (500/4500) can't be tested |
| **Traceroute** | 3 probes per hop, optional reverse DNS |
| **Subnet Sweep** | Ping sweep of a CIDR or range (`10.0.0.0/24`, `10.0.0.1-254`, `10.0.0.10-10.0.1.20`) with hostnames, MAC and vendor, ping-silent devices from the ARP table, optional management-port check, and an **Inventory** JSON for /mapnetwork (see below) |
| **DNS Lookup** | Any record type, or **All** at once (A/AAAA, MX, TXT, CNAME, NS, SOA, SRV, CAA, PTR, DS, DNSKEY) via the system resolver, Cloudflare, Google, Quad9 or the domain's authoritative server, or **Compare all** side by side to spot split DNS and stale caches |
| **Advanced** | Mail and domain DNS health for a domain — validates, not just detects (see below) |
| **Config Export** | Decodes a UniFi backup (`.unf` / `.unifi`) or SonicWall settings export (`.exp`) on this machine and saves sanitized JSON for the unifidump / mapnetwork skills (see below) |
| **Map** | Network map built from the sweep and any UniFi / SonicWall export: roles by colour, real uplinks where a backup has them, findings, search, pan and zoom, SVG export (see below) |
| **Site Bundle…** (top right) | Wraps the UniFi and SonicWall exports, the sweep inventory and optionally the Results log into one zip with a manifest for the unifidump / mapnetwork skills (see below) |

**Themes:** the **Theme** menu in the footer offers Dark (the default: egui's stock dark look), Light and Miami, remembered between launches (`%APPDATA%\is-alive\settings` on Windows, `~/.config/is-alive/settings` on Linux). Miami puts the panels over a backdrop: a 16-bit synthwave scene drawn by the app (sunset, skyline, palms, a grid drifting toward you), your own picture with a slow pan (only its path is saved; the image never leaves your machine), or none. **Animate** loops the scene and the sunset line under the tabs and pauses while the window isn't focused; turn it off on battery. On the GPU the scene runs at the display's refresh rate (60, 120, 144 Hz…, whatever the OS has the screen set to; vsync paces it): it's drawn at art resolution by a shader into an offscreen buffer and scaled up with hard edges, so the CPU does almost nothing. Without a usable GPU (a software OpenGL such as llvmpipe or Microsoft's Basic Render Driver, OpenGL below 3.1 / ES 3.0, or a shader the driver rejects) it falls back to drawing on the CPU at 10 frames a second. The Theme menu says which is in use and why; `IS_ALIVE_GPU=off` forces the CPU.

Files named on the command line (or opened with **Open with → is-alive**) go straight to Config Export, as if dropped on the window.

Every result view has a **Copy** button for pasting into tickets. The **Results** tab also has a **Filter** box (tool name or output text) and **Export** to CSV, JSON, Markdown or HTML, for the whole log or a single run; Copy, Save and Export all use the filtered set.

CSV export uses each tool's own columns, one row per result, with the run's start time first, so repeated runs stack and filter cleanly in Excel:

| Tool | Columns |
|---|---|
| Ping | target, mode, port, seq, from, status, rtt_ms, ttl, bytes |
| Port Check | host, ip, port, state, service, rtt_ms, error (closed/filtered included) |
| Traceroute | host, ip, hop, address, name, rtt1_ms, rtt2_ms, rtt3_ms, note |
| Subnet Sweep | range, ip, rtt_ms, ttl, mac, vendor, name, open_ports, found_by (alive hosts; `arp` rows have no rtt_ms) |
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

**Ping-silent devices.** Pinging an address on your own segment makes the OS send an ARP request first, and almost nothing refuses ARP. Hosts that answer ARP but drop ping (Windows firewall defaults, many IoT devices and cameras) are listed with *no ping* in the Time column and counted as *ARP only* in the header. Only entries the OS has just confirmed count (Linux `ip neigh` REACHABLE, Windows Reachable), so devices that left minutes ago don't show up. The sweep reads the table every 3 s while it runs. Past a router there is no ARP, so only ping counts there. On Linux this needs `ip` (iproute2); without it the sweep is ping-only. The Inventory marks each host `found_by` `ping` or `arp`.

**Check mgmt ports** adds a TCP connect after the sweep to each live host on 22, 23, 80, 443, 445, 541 (FortiGate), 554 (RTSP), 3389, 4433 (SonicWall), 5000, 8080/8443 (UniFi), 8291 (MikroTik) and 9100 (printers), at the same speed limit. Open management ports are the evidence for telling switches, APs and firewalls from clients. It's off by default: it is a port scan.

**Inventory** saves the run as JSON for `/mapnetwork`'s scan input: this machine's LAN, gateway and WAN, the ports checked, and per host `ip`, `mac`, `vendor`, `private_mac`, `name`, `rtt_ms`, `ttl`, `open_ports`, `is_gateway` and `is_this_machine`. Facts only, no role guesses; the MACs are what join it to `/unifidump`'s `discovered_devices` and uplink MACs.

### Site Bundle

**Site Bundle…** (top right, any tab) gathers what the other tabs have produced into one `<site>.bundle.zip` to attach in Claude:

| File | From | For |
|---|---|---|
| `manifest.json` | site name, known problems, client handling, this machine's LAN/gateway/WAN, what each file is | both skills; its `mapnetwork_preflight` answers /mapnetwork's pre-build question |
| `prompt.txt` | the kickoff prompt, also copied to the clipboard on save | you, or paste it into Claude |
| `unifi.unifidump.json` | the UniFi backup open in Config Export, converted again with the bundle's client setting | /unifidump |
| `sonicwall.json` | the SonicWall export open in Config Export | /mapnetwork (firewall config) |
| `inventory.json` | the last finished Subnet Sweep | /mapnetwork (scan) |
| `results.json` | the Results log, off by default: it holds every run since the app opened, which can include other clients' hosts and domains. The row lists the targets it covers so you can check before ticking it | /mapnetwork |

**UniFi clients** defaults to *anonymized* here: each client keeps its MAC, vendor and switch port (what unifidump needs to spot unadopted gear plugged in as a client) but device names, often people's names, become a token like `client-1a2b3c4d`, stable per MAC; notes and email addresses are dropped. *None* and *full* are the other choices. Saving also copies a kickoff prompt to the clipboard ("Run /unifidump on …, then /mapnetwork …"), so it's one attach and one paste. Everything in the bundle comes from the sanitized exports; raw backups and secrets never go in.

### Map

The **Map** tab draws one picture from what the app has seen, in the spirit of `/mapnetwork`: a tree from the internet down, roles as colours (gateway, switches, access points, servers/NAS, printers/cameras, clients, unidentified), infrastructure first and clients only on request.

- **From a UniFi backup** (loaded in Config Export) it draws the real topology: each device under the switch or AP it uplinks to, with the port, wired or wireless (mesh, bridges). An uplink MAC the backup doesn't describe becomes an *Unidentified upstream device* card; a SonicWall export loaded alongside becomes the gateway, matched by its interface MACs.
- **From a sweep alone** it can't see switch ports, so devices hang under the gateway on dotted *assumed* links, with roles inferred from vendor and open ports (marked *inferred*). **Scan this network** on the empty map runs a sweep and the map fills in as hosts answer.
- **Both**: the sweep is merged into a backup only when it found hardware the backup knows by MAC. A backup read at the office isn't mixed up with the office network; the subtitle says so.
- **Clients** (off by default) puts end devices on the map, up to 10 under each switch or AP and the rest in one *+N more* card, within a 120-card budget; off, each uplink shows a count.
- **Findings**, worst first, only from evidence: telnet open, management only over plain HTTP, firewall management allowed on a WAN interface, devices UniFi lists as each other's uplink, unidentified uplinks, end-of-life models, mixed firmware on one model, devices in the swept range that didn't answer, and Ubiquiti hardware the backup doesn't know. Click one to show its devices; **Copy** for a ticket.

Drag to pan, scroll or pinch to zoom, double-click the background to fit, drag a card to move it (**Re-layout** puts it back), click a card to trace its path to the internet and see its details, with **Ping**, **Port Check** and **Open web page** for it. Search matches name, IP, MAC and vendor; click a role in the legend to fade it out. **Export SVG** saves the map with its findings as a picture for a ticket or site doc.

### Config Export: UniFi

**Open file…** takes a `.unf` (site export or classic autobackup), a `.unifi` (UniFi OS console backup), a `.zip` holding one, or a bare `db.gz`; the format is detected from the content, not the name. Everything runs locally: nothing is uploaded or sent anywhere.

| File | Decrypt | Inside |
|---|---|---|
| `.unf` | AES-128-CBC, UniFi's static key | zip → `db.gz` |
| `.unifi` | AES-256-CBC, UniFi's static key, IV in the first 16 bytes | gzip → tar → `backup/network/db.gz` |

**Save JSON…** writes `<name>.unifidump.json`: the mongoexport-style array (`{"__cmd":"select","collection":…}` markers) that the unifidump skill's `extract.py` reads directly, so hand that file to `/unifidump` instead of the backup.

- **Kept:** `site`, `device`, `networkconf`, `portconf`, `wlanconf`, and `user` (clients) only when **UniFi clients** is *anonymized* or *full* (default *none* here; Site Bundle defaults to *anonymized*). Client hostnames are often people's names; unifidump uses them only for counts and to spot unadopted gear plugged in as a client.
- **Left behind:** every other collection: admins and their password hashes, settings, events, alarms, stats, rogue APs, and so on. The console's own config, TLS keys and Postgres database in a `.unifi` are never read.
- **Removed from what's kept:** any field starting `x_` (UniFi's prefix for stored secrets: WLAN passphrases, device auth keys, WAN/VPN passwords) or named like a password, passphrase, secret, PSK, private key, token, certificate or hash, at any depth.

The summary lists the layers peeled, the collections kept and left behind with counts, and every field removed, so you can check what goes out before sharing it. Names, MACs, IPs, VLANs and topology stay: that is what unifidump maps.

### Config Export: SonicWall

Pick several files at once, or drag them onto the window from any tab. The tab keeps one result per kind, so a UniFi backup and a SonicWall export opened one after the other sit side by side, each with its own **Save JSON…**, **Copy** and **Clear**; opening another file of the same kind replaces only that one.

The same **Open file…** takes a SonicWall settings export (`.exp`, Gen 6 or 7), or one already decoded to text. The file is only base64 and URL-encoding; it's decoded here and turned into JSON for `/mapnetwork`: interfaces (zone, physical/VLAN with tag and parent, IP/mask, gateway, secondary subnet, port MAC, which management protocols are on, comment), zones, address objects and groups, FQDN objects, service objects and groups, access rules (zones, source, destination, service, action, enabled, comment) and NAT rules.

The output is built only from those named fields, so admin and user accounts, passwords, VPN pre-shared keys, RADIUS/LDAP secrets, SNMP communities, certificates and PPPoE logins never leave the file; the summary says how many settings were copied and how many left behind. Comments are kept because they say what a rule or port is for; any that mention a password, PSK, secret, community or token are replaced with a placeholder, and the summary counts them. Interface MACs are what `/mapnetwork` matches against `/unifidump`'s unresolved uplink MACs to name the firewall.

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
