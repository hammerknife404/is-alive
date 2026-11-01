# is-alive

> **Attending a demo?** Start with the plain-language **[follow-along guide](FOLLOW-ALONG.md)**: download, open, and try it step by step. No technical background needed.

Native network tester written in Rust. Single portable `.exe` on Windows, single `.AppImage` on Linux — no installer, no admin rights.

| Tab | What it does |
|---|---|
| **Ping** | ICMP (IPv4/IPv6) or TCP-connect ping, continuous or N count, live latency graph, loss/min/avg/max |
| **Port Check** | Concurrent TCP connect scan — lists, ranges, presets (Common, Windows/AD, Web, Firewall/VPN, Mail, 1-1024) with service names. On Windows a port that times out gets one more try, so a lost packet isn't read as filtered. TCP only: UDP services such as IKE (500/4500) can't be tested |
| **Traceroute** | 3 probes per hop, optional reverse DNS |
| **Subnet Sweep** | Ping sweep of a CIDR or range (`10.0.0.0/24`, `10.0.0.1-254`, `10.0.0.10-10.0.1.20`) with hostnames, MAC and vendor, ping-silent devices from the ARP table, optional management-port check, and an **Inventory** JSON for /mapnetwork (see below) |
| **DNS Lookup** | Any record type, or **All** at once (A/AAAA, MX, TXT, CNAME, NS, SOA, SRV, CAA, PTR, DS, DNSKEY) via the system resolver, Cloudflare, Google, Quad9 or the domain's authoritative server, or **Compare all** side by side to spot split DNS and stale caches |
| **Advanced** | Mail and domain DNS health for a domain — validates, not just detects (see below) |
| **Config Export** | Decodes a UniFi backup (`.unf` / `.unifi`), SonicWall settings export (`.exp`) or tech support report (`.wri`), or Meraki MX config (Dashboard API `.json`) on this machine and saves sanitized JSON for the unifidump / mapnetwork skills; also reads a `/mapnetwork` map (`.dc.html`, or a zip holding one) back in for the Map tab (see below) |
| **Notes** | Markdown tech notes with a live preview, font and size choice, and emoji; redact by hand, and a loaded backup's secrets are redacted wherever they appear (see below) |
| **Map** | Network map built from the sweep and any UniFi / SonicWall export: roles by colour, real uplinks where a backup has them, findings, search, pan and zoom, SVG export (see below) |
| **Speed Test** | Download from Cloudflare by default (any large file works), or download and upload with Cloudflare's full test, latency, jitter and loss at idle and under load, a bufferbloat grade and a VoIP call-quality score (MOS) (see below) |
| **Subnet Calc** | Subnet calculator for IPv4 and IPv6: enter `10.0.0.0/22`, an address with a netmask (`10.0.0.5 255.255.252.0`) or an ACL wildcard (`0.0.3.255`) for network, broadcast, host range, counts, mask in binary, type (private, CGNAT, link-local…) and the neighbouring blocks; check whether an address is inside; split it into smaller blocks; or enter a range (`10.0.0.5-10.0.1.20`) for the fewest CIDR blocks that cover it. **Sweep** sends it to Subnet Sweep. Works offline |
| **Site Bundle…** (top right) | Wraps the UniFi and SonicWall exports, the sweep inventory and optionally the Results log into one zip with a manifest for the unifidump / mapnetwork skills (see below) |

**Themes:** the **Theme** menu in the footer offers Dark (the default: egui's stock dark look), Light, Miami, and themes after `/mapnetwork`'s: Slate, Phosphor, Blueprint, High contrast, RGB (neon on black with a rainbow line under the tabs) and Paper. Each theme has its own Map colours (the menu shows them as swatches beside each name), and cards and links are tinted in their role's colour, with a soft glow on dark themes. The choice is remembered between launches (`%APPDATA%\is-alive\settings` on Windows, `~/.config/is-alive/settings` on Linux). Miami puts the panels over a backdrop: a 16-bit synthwave scene drawn by the app (sunset, skyline, palms, a grid drifting toward you), your own picture with a slow pan (only its path is saved; the image never leaves your machine), or none. **Animate** loops the scene and the sunset line under the tabs and pauses while the window isn't focused; turn it off on battery. **Glow** adds a soft halo and haze to the bright parts (the sun, neon and windows on the GPU; your picture's highlights either way). **VHS** (on by default, GPU only) plays the scene as if off a worn tape: faint scanlines, red and blue bleeding a hair apart, grain, lines jittering sideways, and every 9 seconds a tracking band rolling down the picture. The scene, a picture's pan and the sunset line run at 10 frames a second, a slight chop that suits the look of a 16-bit scene off a VHS tape and keeps it cheap (the whole app redraws every frame; at the display's rate the Map tab took a third of a CPU core). On the GPU the scene is drawn at art resolution by a shader into an offscreen buffer and scaled up with hard edges, and a repaint between frames (a hover, typing) reuses that buffer. Without a usable GPU (a software OpenGL such as llvmpipe or Microsoft's Basic Render Driver, OpenGL below 3.1 / ES 3.0, or a shader the driver rejects) it's drawn on the CPU instead. The Theme menu says which is in use and why; `IS_ALIVE_GPU=off` forces the CPU.

Files named on the command line (or opened with **Open with → is-alive**) go straight to Config Export, as if dropped on the window.

**Ctrl+1 … Ctrl+9** switch to the first nine tabs and **Ctrl+0** to Results (the footer's **?** lists every shortcut; Speed Test and Subnet Calc, just before Results, have none), copies and saves confirm with a toast, and the window opens at the size it was closed at.

Every result view has a **Copy** button for pasting into tickets. The **Results** tab keeps the 12 newest runs open and folds older ones, **Clear** asks for a second click, and it has a **Filter** box (tool name or output text) and **Export** to CSV, JSON, Markdown or HTML, for the whole log or a single run; Copy, Save and Export all use the filtered set.

CSV export uses each tool's own columns, one row per result, with the run's start time first, so repeated runs stack and filter cleanly in Excel:

| Tool | Columns |
|---|---|
| Ping | target, mode, port, seq, from, status, rtt_ms, ttl, bytes |
| Port Check | host, ip, port, state, service, rtt_ms, error (closed/filtered included) |
| Traceroute | host, ip, hop, address, name, rtt1_ms, rtt2_ms, rtt3_ms, note |
| Speed Test | server, ip, phase (idle/download/upload), mbps, mb, seconds, streams, probe, probes, lost, latency_avg_ms, latency_min_ms, latency_max_ms, jitter_ms, loss_pct, added_ms, mos |
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

### Speed Test (VoIP readiness)

Runs against Cloudflare's speed test service (`speed.cloudflare.com`, anycast, so the nearest Cloudflare data centre answers; the header names it and your ISP). Three phases:

1. **Idle**: 40 latency probes, 10 a second.
2. **Download**, then **Upload**: parallel HTTPS streams (4 by default, up to 16) for the set duration (10 s by default), probing latency the whole time. The first 20% (1 to 3 s) is left out of the result: TCP slow start and buffers filling, not the line's rate.
3. Results: Mbps each way, and for each phase average/min/max latency, **jitter** (mean difference between consecutive replies, as VoIP tools and RFC 3550 measure it), **loss**, the latency **added** under load and an estimated **MOS**.

| Figure | Meaning |
|---|---|
| **Bufferbloat** | Latency added under load, graded A+ (<5 ms), A (<30), B (<60), C (<200), D (<400), F. B or worse gets a finding: calls break up whenever someone fills the line; the fix is smart queue management (SQM: fq_codel / CAKE) or bandwidth management on the firewall shaped just below the line rate |
| **VoIP MOS** | Simplified ITU-T G.107 E-model (G.711) from round-trip latency, jitter and loss: 4.34+ excellent, 4.03 good, 3.6 fair, 3.1 poor. Shown at idle and under load |
| **VoIP targets** | latency < 150 ms, jitter < 30 ms, loss < 1%; anything over in any phase is called out |

**Server:** **Download a file** is the default, starting on Cloudflare's own 25 MB download (`speed.cloudflare.com/__down`, from the nearest data centre). Any large file's direct link works too, your own server's or a client's. Or pick Cloudflare's full speed test (download and upload, and it reports which data centre and ISP the test ran through). A file is fetched again and again on each stream. It's download only, since file hosts don't take uploads, and latency is measured to 1.1.1.1 so idle latency, jitter and bufferbloat read your line rather than the path to the file. **Upload to a URL** is the other way round: it posts about 25 MB at a time, for the test's duration, to any URL that accepts uploads and throws them away (your own server, or a LibreSpeed server's `…/backend/empty.php`). It's upload only, with latency to 1.1.1.1 too, and useful when Cloudflare is rate-limiting you. Both have a **Presets** menu beside the link: free public servers by region (US East, Central and West, Canada, Europe) from Cloudflare, Hetzner, Vultr, OVH, ThinkBroadband and LibreSpeed hosts (Clouvider, Sharktech, RackGenius), all checked in November 2026. Free servers come and go: if one fails or is slow, pick another nearby. Before the transfers start (during the idle latency phase), the test tries the server's IPv4 and IPv6 addresses at once. A network that hands out IPv6 but can't route it would otherwise make every stream wait seconds before falling back, inside the timed part; then the test runs over IPv4 only and says so. A machine with no IPv6 at all just uses IPv4, with no note. A link to the page a file is offered on, rather than the file, or a site behind a bot check, is reported as such. Cloudflare's downloads are asked for in 25 MB pieces: it refuses large single requests with a 403.

Latency is ICMP to the server's address. If 4 pings get no reply (ICMP blocked), it switches to TCP connects to port 443 and says so; loss is less reliable that way. A failed direction says why, and that web filters (SonicWall CFS, DNS filtering) often block speed test sites. The test saturates the line while it runs, and a gigabit line moves over 1 GB each way, so mind metered links. The full scan (the **is-alive** button) leaves it out: Cloudflare rate-limits back-to-back tests, so run it from its own tab.

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
| `meraki.json` | the Meraki export open in Config Export | /mapnetwork (firewall config) |
| `inventory.json` | the last finished Subnet Sweep | /mapnetwork (scan) |
| `results.json` | the Results log, off by default: it holds every run since the app opened, which can include other clients' hosts and domains. The row lists the targets it covers so you can check before ticking it | /mapnetwork |
| `map.dc.html` | the `/mapnetwork` map open in Config Export, from an earlier visit; the prompt asks for it to be updated rather than rebuilt, under its original name | /mapnetwork (update) |
| `notes.md` | the Notes tab, off by default, sanitized (the row says how many values were redacted) | /mapnetwork (context and findings) |

**UniFi clients** defaults to *anonymized* here: each client keeps its MAC, vendor and switch port (what unifidump needs to spot unadopted gear plugged in as a client) but device names, often people's names, become a token like `client-1a2b3c4d`, stable per MAC. *None* and *full* (real device names) are the other choices; client notes, email addresses and phone numbers are dropped in every mode. Saving also copies a kickoff prompt to the clipboard ("Run /unifidump on …, then /mapnetwork …"), so it's one attach and one paste. Everything in the bundle comes from the sanitized exports; raw backups and secrets never go in.

### Map

The **Map** tab draws one picture from what the app has seen, in the spirit of `/mapnetwork`: a tree from the internet down, roles as colours (gateway, switches, access points, servers/NAS, printers/cameras, clients, unidentified), infrastructure first and clients only on request.

- **From a UniFi backup** (loaded in Config Export) it draws the real topology: each device under the switch or AP it uplinks to, with the port, wired or wireless (mesh, bridges). An uplink MAC the backup doesn't describe becomes an *Unidentified upstream device* card; a SonicWall export loaded alongside becomes the gateway, matched by its interface MACs.
- **From a sweep alone** it can't see switch ports, so devices hang under the gateway on dotted *assumed* links, with roles inferred from vendor and open ports (marked *inferred*). **Scan this network** on the empty map runs a sweep and the map fills in as hosts answer.
- **Both**: the sweep is merged into a backup only when it found hardware the backup knows by MAC. A backup read at the office isn't mixed up with the office network; the subtitle says so.
- **From a /mapnetwork map**: open the `.dc.html` that `/mapnetwork` made (or a zip, such as the site bundle, you saved it into) in Config Export and the Map tab redraws it in this app's style. Only the map's data block is read, as plain values; the page's code never runs. Roles come from each device's kind (firewall, switch, AP, server…), the map's own zone, site and VLANs become details and chips, each device hangs off the first link that reaches it from the internet (any other link is listed in its details, and a device with no link up is drawn as *assumed*), and its findings fill the band. While a map is loaded it *is* the map: a UniFi backup or SonicWall export loaded too is ignored here, and a sweep joins only when it found hardware the map knows by MAC, marking which devices answered.
- **Clients** (off by default) puts end devices on the map, up to 10 under each switch or AP and the rest in one *+N more* card, within a 120-card budget; off, each uplink shows a count.
- **Findings**, worst first, only from evidence: telnet open, management only over plain HTTP, firewall management allowed on a WAN interface, devices UniFi lists as each other's uplink, unidentified uplinks, end-of-life models, mixed firmware on one model, devices in the swept range that didn't answer, and Ubiquiti hardware the backup doesn't know. Click one to show its devices; **Copy** for a ticket.

A note above the map says it's built only from what the app can see and points to Site Bundle → /mapnetwork in Claude for an accurate one; its **×** closes it for good (the footer's **?** keeps the tip).

Drag to pan, scroll or pinch to zoom, double-click the background to fit (or **Fit** in the corner; **Center** glides the middle of the map back to the middle of the view without changing the zoom), drag a card to move it (**Re-layout** puts it back), click a card to trace its path to the internet (it lights up in its colours, with dots flowing away from the internet) and open it in the panel on the right. **Ctrl+click** (⌘ on a Mac) adds cards to the selection or takes them out: each one's path lights up, dragging one moves them all, and **Copy all** copies them. **📌 Pin** a card (in its panel, or right-click it on the map) to keep it selected while you click others; Esc and plain clicks leave pinned cards be, a pinned card holds its place in the panel as others come and go (drag its grip to move it), and the map marks them with the thumbtack in the current emoji style. Search matches name, IP, MAC and vendor; click a role in the legend to fade it out. **Export SVG** saves the map with its findings as a picture for a ticket or site doc.

**Clients** (end devices): **Off** puts a count on each switch or AP; **Some** shows up to 10 under each (fewer on a big map) and folds the rest into a **+N more** card, whose **Show all N** gives each of them a card (**Fold clients** on the switch or AP puts them back); **All** gives every client a card, however many there are (a big site can mean hundreds: **Zones** keeps them tidy).

**Layouts**, as `/mapnetwork` has them (the choice is remembered): **Cluster** (the default: a tree with end devices stacked in columns), **Tree** (one row per level, every card side by side; can get wide), **Zones** (one box per role; links from one device into a box fold into one line with a ×N count, and links inside a box show when you select one end) and **Sites** (one box per site, each holding its part of the tree; the default when the map has two or more sites).

**Sites:** every map has them. A `/mapnetwork` map's come from its SITES (one without SITES is one site, named after the map), a multi-site UniFi controller backup puts each device at its own site, and anything else is one site: the controller's site name, the firewall's name, or *This network*. Click a site's title on the map (▾) to collapse it to one card, which hangs where the site does, lists what's inside, carries its flags and takes everything below the site; click again (▸) to open it. The left panel's **Sites** list has the same toggle on each site, plus **Collapse all** and **Expand all**; click a site's name to select its cards (lighting the whole group's connections) and bring its box into view. Right-click a card to collapse or expand its site. Collapsed sites stay collapsed through rebuilds and in every layout, and a pinned card inside one keeps its pin for when the site opens.

**Updating:** the map rebuilds by itself whenever what it's drawn from changes: as a sweep finds hosts, when a backup, export or map is loaded (or loaded again) in Config Export, and on a host-info refresh. **⟲ Reload** reads the map's files from disk again (the UniFi backup, SonicWall export and `/mapnetwork` map in use) and rebuilds, for when a file changed after it was loaded. Rebuilds keep pins, the selection, the panel's order, moved cards and card results. A pinned card that's no longer in the data isn't dropped: it stays pinned, drawn in red with a dashed outline and marked *Not in the latest data*, so you can check it (its Ping, Ports and Trace still work) before unpinning it or taking it out with ×.

The panels follow `/mapnetwork`'s. On the left: the **sites** (see above), the **roles** with counts (click one to fade it out) and the **action items**, worst first (click one to show its devices); **‹** hides the panel and puts the roles back on the canvas, **›** brings it back. On the right, each selected card stacks with its role, **Action items** (its flags and the findings that name it), **Details** (IP, MAC, open ports, what saw it, then everything the source said), **Connections** (its uplink and what hangs off it, each a link to that card) and a row of buttons: **Ping**, **Ports** and **Trace** run a quick check right in the card, **Web** opens its web page and **Copy** copies it. Each check's condensed result (`4/4 replies · avg 0.34 ms · 0% loss`, `Open: 22, 80, 443 · 12 closed`, `3 hops · 1.2 ms to 10.0.0.251`) shows under **Results**; click it for every reply, port or hop, **⟲** to run it again, **■** to stop it, **×** to clear it, or **Open in tab** for a full run in that tool's tab. Up to three checks run at once and the rest wait their turn. They're spot checks: they stay with the card for the session but don't go to the Results tab or exports. × takes a card out of the stack; drag a card's grip (⠿ at its top left) to move it up or down. Cards glide to their new places when the stack changes (one dragged, one added on top, one taken out). The panel is resizable. While a **Scan** runs, a progress bar under the toolbar shows hosts probed, rate and time left.

### Config Export: UniFi

**Open file…** takes a `.unf` (site export or classic autobackup), a `.unifi` (UniFi OS console backup), a `.zip` holding one, or a bare `db.gz`; the format is detected from the content, not the name. Everything runs locally: nothing is uploaded or sent anywhere.

| File | Decrypt | Inside |
|---|---|---|
| `.unf` | AES-128-CBC, UniFi's static key | zip → `db.gz` |
| `.unifi` | AES-256-CBC, UniFi's static key, IV in the first 16 bytes | gzip → tar → `backup/network/db.gz` |

**Save JSON…** writes `<name>.unifidump.json`: the mongoexport-style array (`{"__cmd":"select","collection":…}` markers) that the unifidump skill's `extract.py` reads directly, so hand that file to `/unifidump` instead of the backup.

- **Kept:** `site`, `device`, `networkconf`, `portconf`, `wlanconf`, and `user` (clients) only when **UniFi clients** is *anonymized* or *full* (default *none* here; Site Bundle defaults to *anonymized*). Client hostnames are often people's names; unifidump uses them only for counts and to spot unadopted gear plugged in as a client. Client notes, email addresses and phone numbers are dropped in every mode.
- **Left behind:** every other collection: admins and their password hashes, settings, events, alarms, stats, rogue APs, and so on. The console's own config, TLS keys and Postgres database in a `.unifi` are never read.
- **Removed from what's kept:** any field starting `x_` (UniFi's prefix for stored secrets: WLAN passphrases, device auth keys, WAN/VPN passwords) or named like a password, passphrase, secret, PSK, private key, token, certificate or hash, at any depth.

The summary lists the layers peeled, the collections kept and left behind with counts, and every field removed, so you can check what goes out before sharing it. Names, MACs, IPs, VLANs and topology stay: that is what unifidump maps.

### Config Export: SonicWall

Pick several files at once, or drag them onto the window from any tab, at once or one after another. Each file gets its own sub-tab (*kind · name*) with **Save JSON…**, **Copy** and **Clear**; the file opened last is shown, and opening the same file again replaces its tab. Past six tabs the oldest fold into a **n more** menu. The Map, Site Bundle and Notes use the file of each kind opened last; Notes redacts the secrets of every open file.

The same **Open file…** takes a SonicWall settings export (`.exp`, Gen 6 or 7) or one already decoded to text. The file is only base64 and URL-encoding; it's decoded here and turned into JSON for `/mapnetwork`: interfaces (zone, physical/VLAN with tag and parent, IP/mask, gateway, secondary subnet, port MAC, which management protocols are on, comment), zones, address objects and groups, FQDN objects, service objects and groups, access rules (zones, source, destination, service, action, enabled, comment) and NAT rules.

It also takes a **tech support report** (`.wri`, the file SonicOS saves from its Tech Support Report; tested on SonicOS 7.3). A TSR is a readable report rather than settings, so it has its own reader that gives the same JSON: model, serial and firmware; interfaces (zone, VLAN and parent, IPv4 address and mask, WAN gateway, port MAC, IPv4 management protocols, shut-down ports, comment); zones and their security type; address objects (host, network, range, FQDN) and groups; access rules (zones, source, destination, services, allow/deny/discard, enabled, comment) and NAT policies. It also reads the **devices the firewall heard on each port** (*Advanced Switching → L2 Discovery*): an LLDP neighbour (switch, AP, phone) is plugged into that port, so it goes on the Map under the firewall with a real link labelled with both ports ("X0, its Port 24"), and a UniFi device whose uplink was only assumed gets that real link. LLTD neighbours are Windows hosts: they go on as clients with an assumed link, since LLTD crosses switches, and without their computer names, which are often people's names. The report's ARP cache and DHCP bindings are options when it's downloaded and are off by default; they aren't read yet. Only those sections are read, field by field; users, VPN, RADIUS/LDAP, SNMP, certificates, licences, registration codes, logs and diagnostics are skipped whole. Service objects aren't read from a report (rules name their services).

The output is built only from those named fields, so admin and user accounts, passwords, VPN pre-shared keys, RADIUS/LDAP secrets, SNMP communities, certificates and PPPoE logins never leave the file; the summary says how many settings were copied and how many left behind. Comments are kept because they say what a rule or port is for; any that mention a password, PSK, secret, community or token are replaced with a placeholder, and the summary counts them. Interface MACs are what `/mapnetwork` matches against `/unifidump`'s unresolved uplink MACs to name the firewall.

### Config Export: Meraki MX

Meraki keeps no config file on the box: its config is what the Dashboard API returns. [`scripts/meraki-export.ps1`](scripts/meraki-export.ps1) saves an organization's into one JSON file (read-only GET calls; the API key comes from `MERAKI_DASHBOARD_API_KEY` or a hidden prompt and is never written):

```powershell
.\meraki-export.ps1 -Org "Harbor Dental" -Out harbor.meraki.json
```

Open that file (or a zip of one JSON file per API call, as other backup scripts save it) in Config Export. Tables are recognised by their shape, wherever they sit: organization, networks, devices (MX models, serials and firmware go in the summary), VLANs with their DHCP reservations (or the single LAN), appliance ports, L3 / inbound / cellular firewall rules in order, L7 rules, port forwarding, 1:1 and 1:many NAT, static routes, site-to-site VPN mode, hubs and subnets, third-party VPN peers, WAN uplink settings and status, and MX Wi-Fi SSIDs. A file in a zip folder named after a network is tagged with that network. Pre-shared keys, VPN, RADIUS and SNMP secrets and PPPoE passwords are never copied (Notes redacts them, as with the other configs); rule comments and names that mention a password or key are withheld. The raw export holds those secrets, so share the converted JSON, not the export.

### Notes

The **Notes** tab is a Markdown scratchpad for the visit: headings, lists that continue when you press Enter, task boxes you tick in the preview, quotes, code and **bold**/*italic*. The top bar is for writing: **Edit / Split / Preview** (it opens in Edit); formatting and an emoji menu; **Time** (date and time with the time zone and where the clock syncs from: `w32tm` on Windows, chrony or timesyncd on Linux) and **Host info** (this computer's name and AD domain, LAN, MAC, gateway and WAN); **Redact**; and the font and size on the right. Fonts are egui's Sans and Mono plus the log-friendly monospace fonts the computer already has (Consolas, Cascadia Mono, Lucida Console, Courier New on Windows; JetBrains Mono, DejaVu Sans Mono, Liberation Mono, Noto Sans Mono, Fira Mono, Source Code Pro, Ubuntu Mono where installed); sizes run 8 to 32. The bottom bar is for what goes out: word count, what's redacted, autosave, and **Open…**, **Copy**, **Raw copy**, **Save .md…** and **Clear**. **Open…** reads a text or config file (up to 5 MB; UTF-8 or UTF-16) in a tab beside Notes: read-only, line numbers, no wrapping, select and copy. It's never saved, redacted or bundled. In **Split**, drag the divider to give either side more room (20–80%; double-click for half and half). Font, size and split are remembered. Emoji are drawn a size up, in the style picked at the top of the emoji menu (it stays open while you pick), in any theme: **Match theme** (monochrome; neon in Miami and RGB), **Monochrome**, **Colour** (each emoji in its own colour: faces yellow, ✅ green, ❌ red; egui draws one colour per glyph, so not full-colour art) or **Miami neon**. Hovering an emoji in the menu shows what it's for and its Markdown shortcode (`:white_check_mark:`, `:fire:`…); Preview draws those codes as the emoji. [`assets/notes-format-test.md`](assets/notes-format-test.md) uses every element and a spread of emoji, each line saying how it should look: paste it into Notes and switch to Split to check a build offline. [`assets/notes-debug/`](assets/notes-debug/) goes further, for debugging: `notes-debug.md` is a checklist to paste into Notes that walks through every feature (views, fonts, formatting buttons, lists that continue, task boxes, emoji and shortcodes, Time and Host info, redaction by hand and from a config, what Copy / Raw copy / Save / Bundle / Clear send out, odd text), and the `viewer-*` files exercise **Open…**: CRLF and tabs with a long line, a UTF-8 BOM, UTF-16 LE and BE, and a binary file it should refuse.

Notes aren't scanned for secrets: what's redacted is what you mark, so nothing ordinary gets mangled. **Redact** (Ctrl+R) redacts the selected text, there and wherever else it appears; with the cursor on a redacted value the button turns into **Unredact**. The **secrets of a loaded backup or export** (every password, PSK, RADIUS secret and key the UniFi or SonicWall converter strips) are held in memory, never saved, and redacted wherever they appear, as whole words, so the staff Wi-Fi password can't slip into a note. Each becomes a `[🔑 REDACTED: …]` marker. **By hand (n)** lists hand picks and kept values with × to undo; hand picks live only in memory, so after a restart the notes hold their markers. Autosave (every couple of seconds, beside the settings), **Copy**, **Save** and the Site Bundle's **Tech notes** get the redacted text; **Raw copy** (two clicks) copies it as written. The Site Bundle's **Known problems** field works the same way, with its own **Redact** button.

### Privacy and security

Built for sites with regulated data (HIPAA): zero trust and as little as possible.

- **Nothing leaves on its own.** Launching the app sends nothing. Traffic happens only when you run something: the public-IP lookup (one DNS query to OpenDNS) on **Refresh** or a full scan, the DNS and domain-health queries you start, MTA-STS/RDAP over HTTPS with the OS trust store, and the Speed Test's transfers to Cloudflare. No telemetry, no update checks, no accounts.
- **Sanitized on this machine before anything is saved or shared**: UniFi exports (secret fields stripped; client notes, email and phone always dropped; client names only when you choose *full*), SonicWall exports (an allowlist of fields; comments redacted), tech notes and Known problems (what you mark, plus the loaded configs' secrets).
- **Clipboard.** On Windows, everything the app copies is marked to stay out of clipboard history (Win+V), cloud clipboard sync and clipboard managers. When the app closes, it clears the clipboard if it still holds the app's last copy (something you copied elsewhere since is left alone).
- **At rest** there are only the settings file (theme and backdrop picture path) and `notes.md`, which only ever holds the sanitized text. The Results log lives in memory and is gone when the app closes.
- **Input files are untrusted.** Only regular files up to 2 GB are read; decompression is capped at 2 GB per layer and document nesting at 64 levels, so a crafted backup is refused rather than exhausting memory or the stack. The parsers are tested against truncated, bit-flipped and random input.
- **Input is bounded.** Text fields take at most 255 characters (2,048 for a link, 1 MB of text in Notes, 20,000 in Known problems), so a pasted log can't swell a field.
- **Nothing it writes can run.** Every save keeps the type it offers: a name typed as `notes.bat` or `report.xlsm` saves as `notes.bat.txt`, so no export becomes a script or a macro-enabled Office file. CSV cells that start with `=`, `+`, `-`, `@`, `|`, a tab or their full-width forms get a leading `'`, so Excel shows a hostile hostname or TXT record as text instead of running it as a formula. **Open…** in Notes only displays a file; Markdown links are shown, never followed.
- **Notes stay private on disk.** `notes.md` and the allow list are written readable by this user only (mode 600 on Linux and macOS; the profile folder on Windows).
- **System tools** (`ip` and `arp` for the neighbor table, `ping` as a fallback) run only from `/usr/sbin`, `/sbin`, `/usr/bin` or `/bin` with a C locale, never from a PATH lookup. On Windows the app uses the OS APIs instead.

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

### Screenshots for UI work

`IS_ALIVE_SHOTS=<dir>` makes the app visit every tab, save `<dir>/NN-<tab>.png` of each, and close.
For the Map with data, make the fictional demo map and open it at the same time:

```sh
python3 scripts/demo-map.py demo.dc.html
IS_ALIVE_SHOTS=shots ./is-alive demo.dc.html
```

`IS_ALIVE_SHOTS_SELECT=<card id>` picks the card selected for the Map panel shot, and `IS_ALIVE_SHOTS_CLIENTS=some` (or `all`) starts the Map with clients shown. On a pull request that touches `src/`, the **screenshots** workflow runs the tour in Dark and Light and attaches the images as an artifact.

### Linux AppImage

```sh
packaging/linux/build-appimage.sh
# -> target/appimage/is-alive-x86_64.AppImage
```

Downloads `appimagetool` on first run (or set `APPIMAGETOOL=/path/to/appimagetool`). Only glibc is linked; GL, X11/Wayland and xkbcommon load from the host at runtime (`libxkbcommon-x11`, which stock Ubuntu lacks, is bundled), so the AppImage runs on any desktop distro with glibc ≥ the build host's (CI builds on Ubuntu 22.04 → glibc 2.35).

On Linux, ICMP uses unprivileged ping sockets (`SOCK_DGRAM`/`IPPROTO_ICMP`) — no root, no capabilities. That needs your group inside `net.ipv4.ping_group_range`, the default on Fedora, Ubuntu, Arch and most systemd distros. If it's not, the app falls back to the system `ping` command.

CI builds the `.exe` and the `.AppImage` on every push/PR (Actions artifacts).

Versions: a release with new features bumps the minor number (0.11.0); fixes and polish between feature releases bump the patch (0.11.1, 0.11.2, …).

To release, bump `version` in `Cargo.toml` (and `Cargo.lock`) in a PR and merge it to `main`: CI tags `v<version>`, publishes a GitHub release with both binaries and checksums, and generates release notes from the PRs merged since the last release. Pushing a `v*` tag by hand does the same, as long as it matches `Cargo.toml`.

On other platforms (macOS etc.) ICMP falls back to the system `ping` command (dev/testing only).
