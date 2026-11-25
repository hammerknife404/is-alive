---
name: "mapnetwork"
description: Manual-trigger skill for the `/mapnetwork` command. Builds or updates an interactive, self-contained HTML network topology map for a client site from scans, device lists, UniFi/Hostifi/Meraki screenshots, firewall configs, or plain notes. Only trigger on explicit `/mapnetwork` invocation — do not auto-fire on general network-topology or diagramming questions.
---

# /mapnetwork

Turns raw site info (nmap dumps, UniFi/Hostifi controller screenshots, device
lists, firewall configs, notes) into a self-contained interactive `.dc.html`
topology map — pan/zoom, search, collapsible sites, zone filtering, flow-trace
on tap, a findings band, light/dark + Themes menu, auto-layout. One map per client, one file:
the engine is inlined directly in the `.dc.html`, nothing else to ship
alongside it. Opens in any modern browser with internet access (fonts + React
load from CDN inside the file itself).

Trigger: **only** on explicit `/mapnetwork`. Never auto-invoke this for generic
"how do I diagram my network" questions — that's a normal conversation.

`/unifidump` is the only upstream feeder, and it never starts a build on its
own: it stops at its summary and offers a map. A "yes" to that offer counts
as an explicit `/mapnetwork`. So does `/mapnetwork` sent alongside a UniFi
export — in that case run `/unifidump`'s extraction first, then continue here
at Step 1 without re-asking for input you already have.

## What a good map is

A map is a **decision tool for a technician**, not an inventory dump. The test
is whether someone who has never seen the site can open it and, within ten
seconds, say what the shape of the network is and what's wrong with it. Two
failure modes to actively avoid, in order of how often they happen:

1. **Too many nodes.** 500 device cards is not a map, it's a haystack — and it
   renders slowly enough on a real laptop to feel broken. Obey the node budget
   below.
2. **Colour with no meaning.** Zones are the map's only semantic colour channel.
   If they're spent on geography, or auto-generated as twenty pastel hues off a
   colour wheel, the legend and the filter both become noise.

## The node budget — 120 nodes, hard

**Never emit more than 120 NODES in one map file.** Aim for 40–90. Every node
past that costs legibility and render speed, and none of them are what the
client is paying for.

Spend the budget in this order, and stop when it's gone:

1. **Gateways, firewalls, routers** — always, every one.
2. **Switches** — always, every one.
3. **Access points** — always, every one.
4. **Servers, NAS, storage, hypervisors** — always.
5. **Discovered/unadopted infrastructure** (see the schema guide) — always.
6. **Client devices** — only when explicitly asked (see below).

Steps 1–5 essentially never exceed 120 for a single client, even a 20-site one
(the worked Contoso Health example: 23 sites, 81 nodes). If they somehow do,
say so and ask the user how to scope before you build — don't silently truncate
and don't silently split into a second file.

## Client devices are OFF by default

Workstations, phones, printers, IoT, guest devices, and the whole UniFi/DHCP
client table are **not on the map unless the user asks for them.** A controller
export with 1,400 clients is a spreadsheet, not a topology. Ask directly in
Step 1 for every new map — don't wait for the user to think to request it.

- Default: infrastructure only. Note the omission in `META.subtitle`
  (e.g. `"… · clients not mapped"`) so nobody mistakes it for a complete
  device census, and add an ACTIONS entry only if the client data itself
  revealed something (see `possible_infra_clients` below).
- On request ("add the clients", "map the workstations", "I want to see
  what's on the Oscar switch"): add them for **that scope only** — the one
  site or the one switch they asked about — following the "Client devices
  (CD) — cap at 10, collapse the rest" rule in the schema guide, and still
  inside the 120 budget. If the ask blows the budget, map the requested
  scope and say what you left off.
- Never add clients to a multi-site map wholesale. That's the single change
  that turns a working map into an unusable one.

The exception that is always worth mining, even with clients off the map: a
client-table entry that is really **infrastructure** (see
"`possible_infra_clients`" below). Pull those onto the map as real nodes.

## Zones are roles. Sites are places.

- **`ZONES`** = role / segmentation / trust: `gateway`, `distribution`,
  `access`, `servers`, `voip`, `workstations`, `iot`, `guest`, `external`.
  Adapt to the site's real segmentation; **never** use zones for geography.
  Cap at ~8, and take the colours from the approved palette in the schema
  guide — never generate a hue ramp per site.
- **`SITES`** = geography: buildings, campuses, cities, suites. One entry per
  physical location, `site:` on every node. Sites render as collapsible
  containers; a collapsed site is one summary card, and the engine folds its
  links into a single trunk per neighbour.

A 23-site controller export is therefore **one file** with 23 `SITES`, not 23
files and not 23 zones. Multi-site maps open with every site collapsed, so the
first thing a tech sees is the shape of the estate; they expand what they care
about (sidebar row, container title, or the collapsed card itself). Don't ask
the user how to handle multiple sites — this is the answer.

## Cards are a fixed box

Every node renders in the same 212×112 card — the engine pins both axes and
clips overflow, so nothing you write can make one card taller or wider than its
neighbours. Author to fit it:

- **`kind`** — ALL CAPS, **≤ 18 characters**. Longer truncates to an ellipsis.
  `SONICWALL INFERRED`, not `SONICWALL DEVICE (inferred)`.
- **`label`** — clamps to 2 lines. Put the distinguishing part first; a name
  like `Alpha - Exec Suites - IDF Right` reads better as `Exec Suites — IDF R`
  with the full string in `details`.
- **`chips`** — one line, ~2 short chips. The rest belongs in `details`.

## Step 0 — figure out new map vs update

Before asking anything else, check context:
- Did the user attach or reference an existing `<Client>.dc.html`? → **update path**.
- Otherwise → **new map path**.

If ambiguous (e.g. they just typed `/mapnetwork` with no attachment and no prior
map mentioned in this chat), ask one quick question: new site or updating an
existing map. Don't stall on this — one question, move on.

## Input from an is-alive site bundle

A zip whose `manifest.json` says `"kind": "is-alive site bundle"` comes from
the is-alive app, made on the technician's machine. Read `manifest.json` first:

- **Skip Step 1.** `mapnetwork_preflight` already answers it (site name, scan,
  firewall config, known problems, existing map, clients on or off). Don't ask
  again; `existing_map` set (the bundle's `map.dc.html`) means the **update
  path**, saved under that name.
- **A `map.dc.html` is-alive wrote itself** (its SITE DATA starts with the
  comment *Exported from is-alive's Map*, and `META.subtitle` ends
  *from is-alive <version>*) is the app's own quick map, not one you drew. It
  holds sites, zones, uplinks and ports, the app's findings and the
  technician's card notes, and clients only if the app drew them (it stops at
  120 cards, leaving out clients first and then the cards furthest from the
  internet, and says how many of each in the subtitle). A subtitle that starts
  *Plan —* or *Training map —* is not what was found on site: those cards were
  drawn by hand. A `drawn` chip marks such a card (its link dashed): keep the
  chip, and never report a drawn card as discovered. A dashed `wired` edge is an uplink
  the app inferred, not one a backup showed; a device's second link is a line
  of text in its notes, since a card has one uplink. Treat it as **observed**,
  not enforced config: take the update path, but redo the topology from the
  bundle's configs and inventory where they say more, and rewrite its findings
  in your own words.
- **Files and their tier** (Step 3's tiers): `unifi.unifidump.json` goes
  through /unifidump first; `sonicwall.json` / `meraki.json` are **enforced
  config**; `inventory.json` is **observed** (a ping/ARP sweep). `notes.md`,
  `card-notes.json` and the manifest's `known_problems` are the technician's
  word: context, findings and device notes, never topology on their own.
  They're redacted only where the technician marked a value (and the loaded
  configs' passwords and keys), not scanned: anything in them that still
  looks like a credential or a person's or patient's details may be real, so
  never repeat it in the map, its findings or your reply.
  `results.json` (test runs) becomes a finding only where a run shows a problem
  (loss, a closed port that should be open, a failing mail check). A check
  that **passed** still belongs on the map, as one short `details` line on
  the node it tested (`Ping 0% loss, 1.2 ms avg (is-alive, <date>)`,
  `HTTPS inspection: none`): otherwise the next visit can't tell "checked,
  fine" from "never checked". A `Traceroute` run names the hops from the
  scanner out: the first hop is the LAN's gateway, and the first public
  address is the ISP's edge; use it to say which WAN the LAN leaves by, and
  put the hop list in the internet/WAN node's `details`. A site
  keeps its runs from one visit to the next, so they can span visits: judge
  each check by its **newest** run (`started`); an older failure that a later
  run of the same check doesn't repeat is history, not a finding.
  `Host info` and `Domain join` runs describe the computer is-alive ran on,
  not a device to draw: `Domain join` says how it's joined (`kind`: hybrid,
  domain, entra, registered, workgroup, unknown) and to which `domain`. On a
  client's own PC that names the client's AD domain or Entra tenant; on the
  technician's laptop it's the technician's own, so say which it was before
  using it. A join that's broken (`DeviceAuthStatus` not SUCCESS,
  `AzureAdPrt` NO on a joined PC) is a finding. Host info's `dhcp_server`,
  `lease_obtained`, `lease_expires` (local time on the scanner, or
  `unknown`/`none`), `dns_servers` and `dns_suffix` are what that computer
  was given: context, not findings (put `dns_servers` on the gateway or the
  DNS server's node as a `details` line, `DNS handed out: …`, so the map
  says which resolver the LAN uses); a lease length is a site setting, not a
  problem. A DHCP server that isn't the gateway is normal (a domain
  controller often hands out addresses), and one run can't show a rogue
  DHCP server, so never report either from these rows.
  `Network checks` rows (`check`, `status`, `detail`): a `Captive portal`
  that fails means a portal answered instead of the internet, so that
  run's `HTTPS inspection`, `Clock` and `Path MTU` describe the portal,
  not the site's internet: say so rather than reporting them. `Path MTU`
  `ok` covers *path MTU found* (a router said fragmentation needed; 1492
  on PPPoE is normal); `warn` is *silence above N bytes*, an MTU black
  hole and a finding. Its `detail` names the exact largest packet that
  passed between 1400 and 1500 bytes (*1492 bytes is the largest packet
  that passes (1493 doesn't)*); *nothing from 1400 bytes up* means the
  path is under 1400, not that it was unmeasured. `Wi-Fi` is one snapshot of the network the scanner
  was on: its `detail` names the SSID, the access point's BSSID, band,
  channel and its width, signal and the link's Tx/Rx rates. The BSSID is that access point's radio MAC: match it
  to an AP in the configs or inventory to say which AP the technician was
  near, but never claim roaming or coverage from one snapshot. A weak
  signal there is a note about where the scan ran, not a site finding on
  its own. `Nearby Wi-Fi` lists the other access points heard there (SSID,
  BSSID, channel, signal), those on the connection's channel first: its
  co-channel interference. It's the list the system already had (it says
  how old where it can), reported, not judged: BSSIDs of the site's own
  SSIDs help place APs from the configs, and several strong ones sharing a
  channel can back a channel-planning note, but never call it a fault from
  one list. A `Wi-Fi scan` run (`ssid`, `bssid`, `band`, `channel`,
  `signal_dbm` or `signal_pct`, `connected` = `yes` on the scanner's own
  AP) is the same list from an active scan the technician asked for, so it
  is fresher; read it the same way.
  A `Domain join (others)` run is different: each row is another computer
  on the network, asked without credentials (`host` as typed, `computer`,
  `domain`, `dns_domain`, `dns_name`, `verdict`, `port`, `detail`). A
  verdict starting `AD:` is observed fact: put that domain in the node's
  `details`, and use `computer` as its name when the map has none. `Not
  AD-joined (Entra not checked)` means only that: never write that a
  device is a workgroup machine or has no directory from it. `No answer`
  and `Answers, but doesn't say` are not findings.
- **Several of a kind** (one visit, more than one controller or firewall):
  the first keeps its name and the next are numbered, `unifi-2.unifidump.json`,
  `sonicwall-2.json`, `meraki-2.json` and so on; `manifest.json`'s `files`
  gives each one's `source` (the file it came from), and `firewall_config` is
  then a list. Each `unifi-N.unifidump.json` is **its own UniFi controller**:
  run /unifidump on each and keep each controller's sites its own. Two
  controllers can both have a site called "Default": they are different
  sites, so tell them apart by the source file's name. (Two backups of one
  controller from different days share their sites' ids and their devices'
  MACs: draw each device once, as the newer file has it.) Each `sonicwall-N.json`
  (or `meraki-N.json`) is **its own firewall**: a gateway of its own under the
  internet, with what plugs into it below it. A device goes under the firewall
  whose interface MAC is its uplink MAC, else the one whose LAN subnet holds
  its IP; two sites often share a subnet like 192.168.1.0/24, so where the
  subnet doesn't decide it, go by the controller site the device is at.
- **`inventory.json` hosts:** `found_by` lists every way the host answered
  the sweep: `ping`, `arp` (the scanner's ARP table confirmed it, own segment
  only), `mdns` (it answered the DNS-SD browse) and `ssdp` (it answered the
  SSDP search). Several together are normal (`["ping", "arp", "mdns"]` for a
  printer). `["arp"]` without `ping` means it answered ARP but drops ping (a
  firewall default, not a fault). A list with only `mdns` and/or `ssdp`
  (`rtt_ms` null) is a device silent to ping and ARP that answered the
  multicast browse: a real device on the scanner's own segment (it is only
  listed when it answered from its own address, inside the swept range,
  with `on_scanner_lan: true`; an address merely named in another device's
  answer is never added), often a
  cast target, TV, speaker or IoT device. Draw it like any other host; a
  routed network can never produce one. (Inventories from before 0.33.0 have
  `found_by` as a single string, `"ping"` or `"arp"`: read it as a one-item
  list.) `open_ports` is role evidence for the
  infra test (22/443/8443 on a vendor's gear, 9100 printer, 554 camera;
  telnet 23 alone is not gear when the maker, `web_title` or `dnssd` says
  printer, since many copiers leave it open). It is
  `null` where that host wasn't checked (the default Smart check skips phones
  and laptops: a private MAC, or a laptop or phone network card's maker) and
  `[]` where it was and nothing answered: null is no evidence either way.
  `complete: false` (top level) means the sweep was stopped before it
  finished: hosts it hadn't reached are missing and some ports weren't
  checked, so don't read a missing host as gone since the last visit.
  `web_title` is what the host's own web page calls itself (its `<title>`,
  else its `Server` header; a placeholder title such as "Loading...",
  "Login" or "Redirecting" counts as none, so the `Server` header shows
  instead), read where a management web port answered: a
  strong role and model hint ("UniFi OS", "SonicWall - Authentication", an
  HP printer's page), the device's own claim rather than proof.
  `dnssd` is what the host said over DNS-SD (Bonjour), `null` if it said
  nothing: `services` lists the service types it offers (`_ipp._tcp` and
  `_pdl-datastream._tcp` are printing, `_airplay._tcp` and `_googlecast._tcp`
  cast targets, `_smb._tcp` a file share), and `ty` and `product` are a
  printer's make and model from its TXT record (`null` when it gave none,
  or when one host advertised several different printers). A model belongs
  to the printer service, not necessarily the host: a Mac, a Windows PC or
  a CUPS print server that shares a printer advertises that printer's
  model. Label a node with it only when the rest says printer (a printer
  maker, 9100 open, no `_smb._tcp` or `_airplay._tcp`); on a computer it
  means "shares a printer", worth a detail line. Use the services as role
  evidence; it is the device's own claim, not proof. The browse is
  multicast, so it only reaches the scanner's own segment: only hosts with
  `on_scanner_lan: true` can answer, and a routed host's `null` means
  nothing. `dnssd_browsed` (top level) says whether the browse ran and its answers
  were read (it doesn't with names off, when no swept range is on the
  scanner's segment, or when the sweep was stopped first: then no `dnssd`
  means nothing). `dnssd` also carries `rp`, the printer's queues (`ipp/print`; a
  print server lists one per printer it shares, so several queues are a hint
  it's a computer or server, not the printer), and `pdl`, the page languages
  its printer services take (MIME types: `application/pdf`,
  `application/vnd.hp-PCL`, `image/urf` for AirPrint); both `[]` when not
  given. `_smb._tcp` is a file share (a NAS, a Windows or Mac computer) and
  `_http._tcp` a web page the device advertises.
  `ssdp` is what the host said over SSDP (UPnP discovery), `null` if it said
  nothing: `server` is its `SERVER` header (OS, UPnP stack, often the
  product: a NAS's, a TV's, a router's), `st` the search targets it answered
  for (`upnp:rootdevice`, `urn:schemas-upnp-org:device:MediaRenderer:1` a
  media player or TV, `urn:schemas-upnp-org:device:InternetGatewayDevice:1`
  a router with UPnP on, `urn:dial-multiscreen-org:device:dial:1` a cast
  target), `uuids` its device UUIDs from the `USN` headers (match one device
  across visits by them), and `locations` its description URL, kept as text:
  is-alive never fetches it. Role hints, the device's own claim, not proof.
  An `InternetGatewayDevice` that isn't the gateway is a second router on
  the LAN: worth a finding. `ssdp_searched` (top level) says whether the
  search ran and its replies were read, in the same cases as
  `dnssd_browsed`. It goes out with TTL 1, multicast on the scanner's own
  segment only, so `null` on a routed host means nothing.
  `other_macs` lists any other MAC the address answered from during the
  sweep: two devices on one IP (a duplicate address), or a failover
  mid-sweep. Flag it on that node and in `ACTIONS` (med).
  An `ip` in 169.254.0.0/16 is self-assigned: DHCP didn't answer that
  device. Flag it on the node and in `ACTIONS` (med) with the count and
  "check the DHCP server/scope and the switch port". A client whose `ip` is
  the gateway's, a switch's, an AP's or a server's (a different device, not
  the same `mac`) is an address clash: one `ACTIONS` item (high on the
  gateway's address, else med) naming both nodes. is-alive raises the same
  findings when it reopens the map, client lines of a "+N more" card
  included. `open_ports` includes 21 (`ftp`) when checked: printers and NAS
  often leave FTP on, clear text, worth a low item.
  `vendor` is the registered owner of the MAC prefix. A locally
  administered MAC (the `0x02` bit of the first octet set) is one of two
  things. `private_mac: true` is a phone, laptop or VM's private address
  with nothing to say infrastructure: `vendor` null, not infrastructure.
  `virtual_mac: true` is infrastructure's own virtual address: the
  gateway's (never private, whatever its MAC), or a network-gear maker's
  prefix with that bit set on a host showing gear of its own (telnet,
  541, 4433 or 8291 open, or a web page that names gear; a laptop's 445
  or 3389 doesn't count), such as a SonicWall LAN interface answering on `2e:b8:ed:…` for
  SonicWall's `2c:b8:ed:…`. `vendor` then names that maker (null on a
  gateway whose maker isn't known). Match it to a firewall export's or
  UniFi's interface MAC with that bit cleared. Both are `false` for a
  registered MAC and `null` when `mac` is. (Inventories from before 0.35.0
  have no `virtual_mac` and called every locally administered MAC private,
  the gateway's included: trust `is_gateway` over `private_mac` there.)
  `vendor: "HP"` covers HP printers, HP PCs and, since HP bought Poly, Poly
  desk phones on HP's newer prefixes: decide by `web_title` (a Poly or VVX
  page is a phone, a LaserJet page a printer), `dnssd` and `open_ports`,
  and call it an HP device only when nothing else says. Carrier and voice
  edge boxes (Adtran, EdgeMarc, AudioCodes, Ribbon/Sonus, Patton) are
  network gear, often answering only ARP: the provider's demarc, not a
  client.
  `is_this_machine` is the scanner itself: leave it off the map. `is_gateway`
  is the default gateway. The top-level `scanner` (`lan`, `gateway`, `wan`)
  says where the sweep ran from: the scanner's address and prefix, its
  default gateway, and the public address it left by. Use it to say which
  WAN this LAN leaves by (with several WANs or firewalls, the one whose LAN
  holds `scanner.lan` and whose public address is `scanner.wan`), and put
  `wan` on the internet node (`sub: 'WAN 203.0.113.7'`). `ranges` lists every network swept;
  `on_scanner_lan: false` marks a host on a routed network, where a MAC comes
  only from the host itself (below) — a missing one there is **expected**,
  not an unidentified device.
- **Where a host's `name` and `mac` came from:** `name_from` is `dns` (its
  PTR record), `netbios` (the computer name a Windows, Samba or NAS host
  reports) or `mdns` (a `name.local` an Apple device, printer, cast device or
  Linux host reports); `mac_from` is `arp` (the scanner's own segment) or
  `netbios` (a Windows host past a router reported it). `netbios` and `mdns`
  are the device's own claim: use them as labels and role hints (a
  `Brother-MFC-L2750DW.local` is a printer), never as proof of identity.
- **`client_names`** (in `inventory.json`, and in the inventory's and each
  Meraki file's `summary` in `manifest.json`) says what became of the names
  of phones, laptops and PCs: anything not the gateway that no maker, port
  or web page marks as network gear, a NAS, a printer or a camera. Their
  names are often people's. `full` keeps them; `anonymized` makes each a
  `client-xxxxxxxx` token from its MAC (else its IP), the **same token** the
  UniFi client table uses for that MAC, so match devices across files by it
  or by MAC; `none` leaves `name` null (in Meraki's `fixed_ips`, no `name`).
  `name_from` still says where the real name came from. A token is not a
  name: label the node by role and vendor instead, never invent a person.
- **Every `inventory.json` host field:** `ip`, `mac`, `mac_from`, `vendor`,
  `private_mac`, `virtual_mac`, `name`, `name_from`, `found_by`, `rtt_ms`, `ttl`,
  `open_ports`, `other_macs`, `web_title`, `dnssd`, `ssdp`, `is_gateway`,
  `is_this_machine`, `on_scanner_lan`.
- **`card-notes.json`**: each note names its card by `name`, `ip` and `mac`;
  put it in that node's `details` (and `flags` when it reports a problem).
  Its `inferred` list says what about that card is-alive worked out and did
  not read or measure: `role` (from the vendor, open ports or web page
  alone) or `uplink` (no backup or config says which port it's on).
  Empty means both were read. Treat an inferred value as a lead to confirm
  from the other files, and keep the `inferred` chip on the node until one
  does.
- **Fill `ip` and `mac` on every node** you can (Step 2's schema): is-alive
  reopens the map and matches devices, sweeps and notes by them.

## How is-alive reads the map back

The technician opens your `.dc.html` in is-alive, which redraws it and
merges this visit's sweep and UniFi clients into it. It reads only the
data, by these rules, so write it to be read:

- **Put a role word in every `kind`**: `FIREWALL`, `ROUTER`, `SWITCH`,
  `AP`, `SERVER`, `NAS`, `PRINTER`, `COPIER`, `CAMERA`, `PHONE`, `PBX`,
  `UPS`, `ACCESS CONTROL`, `WORKSTATION`. The role comes from the `kind`
  first, then the zone's key and name, and the `label` last. A `kind` with
  no role word (`TA900`, `IPC-427`) leaves it to the zone.
- **Zone words on re-import:** `gateway`/`edge`/`routing`/`firewall` →
  gateway; `distribution`/`access`/`switch` → switch; `wifi`/`wireless` →
  access point; `servers` → server; `iot`, `ot`, `cameras`, `printers` →
  printers, cameras, IoT; `workstations`/`trusted`/`guest` → clients;
  `voip`/`voice` → voice gear (a PBX, ATA or voice router), **not** a client
  unless its `kind` says `PHONE`; `external` → unidentified unless the
  `kind` names the role. Network words win over voice words, so a "voice
  edge" zone's router is a gateway. A carrier's router: `kind:'CARRIER
  ROUTER'` (or `ISP ROUTER`), never a client zone.
- **`dashed:true` has one meaning on re-import: this link isn't confirmed.**
  Say why in the edge's `label`, since is-alive shows the reason: `·
  reciprocal` (uplink data points both ways), `last seen` (where the
  controller last saw it, not live), or nothing extra (you inferred it).
  A dashed edge whose label names a port is read as wired on that port.
  `wifi` links are wireless whether dashed or not. A tunnel is not dashed:
  `cls:'wired'`, `label:'IPsec to Branch'`.
- **An edge `label` is the parent's port** (`Port 7`, `Gi1/0/7`, `X2`).
  When you know both ends, write the parent's first: `X2 → Port 24`. A
  label that isn't a port (`21 wired`, `VLAN 10 gw`, `LAG`) is kept as a
  note, not a port; put a VLAN after the port (`Port 14 · VLAN 40`).
- **Backbone** (`cls:'backbone'`) from the internet is the WAN; a backbone
  from the firewall's LAN port to the core switch is read as LAN backbone
  with its port: label it with the firewall's port (`X0 → Port 24`).
- **Every node needs an edge into it.** A gateway with none is put on the
  internet; anything else goes under *Not linked*, placement unknown. Hang
  a device whose uplink you can't find under its likely parent with a
  dashed edge rather than leaving it unlinked.
- **A MAC belongs in `mac`** (or the `sub` line). A MAC in `details` counts
  only on a line that is the MAC (`MAC aa:bb:…`); write a candidate's MAC as
  "candidate"/"unconfirmed" so it's never taken for the node's own.
  Speculative infrastructure carries the `inferred` (or `scan-only`) chip:
  is-alive never places a sweep's hosts under it ahead of a confirmed
  switch.
- **"+N more" cards:** one `details` line per device, name first, then
  what's known, `·`-separated: `client-0a1b2c3d · Hikvision · guest`,
  `front-desk · 192.0.2.40`. Each line becomes its own client card with
  that vendor and guest tag. Devices you only count go in one line,
  `…and 121 more`, kept as a count. Never repeat a device on two lines.

## Step 1 — one pre-build ask

Before building (new map or update), send ONE consolidated prompt, even when
input is already in hand (a UniFi export, a /unifidump summary, a pasted scan).
Ask once; never re-ask. A one-word reply ("no", "go") means proceed.

> Anything else for this site before I build?
> - **IP/nmap scan**: finds unmanaged and unadopted devices the controller never saw
> - **Firewall config** (SonicWall etc.): real VLANs, zones, and rules instead of inferred segmentation
> - **Notes on known problems**: go straight into the findings
>
> Also: infrastructure only, or include clients? (default: infra only)

If the client/site name isn't known yet, add it to the same prompt. If the
user doesn't answer the clients question, default to infrastructure only and
say so in `META.subtitle`. Accept any format. Extra files that arrive
mid-build are merged in, not treated as a restart. Your job is translation,
not validation.

## Step 2 — read the schema

Before writing any SITE DATA, read `references/schema-authoring-guide.md` in
full. It defines META/SITES/ZONES/NODES/EDGES/ACTIONS/BANDS, the budget, the
palette, and the auto-layout modes. Don't guess the schema from memory — re-read
it each time; the engine has specific conventions (edge direction =
upstream→downstream, zone colours need dark+light pairs, sites are a separate
field from zones, etc).

Themes: the header has a Themes menu (Dark, Slate, Phosphor, Blueprint, High
contrast, RGB, Light, Paper) next to the ☾ light/dark flip. Themes are defined
in `themeDefs()` in the engine — don't edit them per map. Themes recolour only
the five standard zone keys (core, server, trusted, iot, guest); any custom zone
falls back to its own dark/light pair, so keep using the standard keys where
they fit. `defaultTheme` accepts any theme id.

Default UX (engine-level, applies to every map — do not override per map):
- Default theme is **Dark** (`defaultTheme: 'dark'`), to match is-alive; a theme the viewer already saved in their browser wins.
- The **Findings** band starts **collapsed** on all screen sizes (click its header to open).
- Every sidebar section (Sites, Zones, VLANs, Action items) has a **hide/show** button in its header.
- On desktop the left sidebar collapses to a narrow rail (‹ / ›) and the right detail panel collapses to its header (▴ / ▾), so the map can take the full canvas while the selection stays visible.

## Step 2.5 — Beacon cross-check (when Beacon tools are present)

If Beacon tools are in the tool list (load them with `tool_search` if they
are deferred), read `references/beacon-crosscheck.md` and run it before any
NODES are written: status → resolve the client → learn its `config_kind`
vocabulary → pull its documented infrastructure. That takes 3–4 calls. It
turns inferred infrastructure into identified infrastructure, and it is the
only way documentation drift and undocumented gear become findings.

- **New maps:** always run it.
- **Updates:** run it only when the update adds or re-identifies
  infrastructure.
- **No Beacon tools, or the call errors:** skip it and say so in one line of
  the reply. It never blocks a build.

Beacon reports IT Glue as `authoritative`. For network facts, the conflict
ladder in Step 3 still treats it as a **claimed** source, below the
controller and firewall config.

## Step 3 — build the map

**New map:**
1. Copy `assets/Site Map Template.dc.html` to
   `/mnt/user-data/outputs/<Client_Name>.dc.html`. **No spaces in the
   filename** — use underscores. (A space in the filename breaks the engine's
   self-refetch in some browsers; the map renders as raw source.)
2. Replace **only** the lines between the `SITE DATA` and `END SITE DATA`
   comment markers in the logic class — locate them by the marker text, don't
   exact-string match the equals signs. Both markers are single-line, closed
   `/* ===== … ===== */` comments: keep both marker lines and insert the data
   after the start marker's closing `*/`. On an older map whose start banner
   spans several lines, that `*/` is on the banner's last line, not the
   marker line — splicing after the marker line leaves the whole block inside
   the comment and the map renders empty (Step 3.5 rejects this). Leave every
   line outside the markers untouched — that's the inlined engine, not client
   data.
3. Default to **auto-layout**: omit `x/y/w/h` on every node, `this.BANDS = []`.
   Only hand-position (`x/y/w/h` + BANDS) if the user explicitly wants a fixed
   layout or is recreating a specific physical floor plan.
4. Translate input → schema:
   - Each infrastructure device → a NODE, with `zone` = its role and `site` =
     its location. Skip client devices (see above).
   - Each connection → an EDGE, `from` = upstream (toward internet/gateway),
     `to` = downstream. WAN/gateway links → `cls:'backbone'`. Switch/wired →
     `'wired'`. AP/WiFi clients → `'wifi'` + `dashed:true`.
   - Anything broken, unpatched, unbackedup, single-point-of-failure, or
     otherwise worth flagging → both a node-level `flags[]` entry AND a
     site-wide `ACTIONS` entry with `sev`. **This is the deliverable** — the
     findings band across the top of the map is the first thing the client
     reads, so write each one as a sentence a technician can act on: what's
     wrong, at which device, and what to do about it. Don't skip this because
     the user didn't explicitly ask for a risk register.
   - Order ACTIONS worst-first and be honest with `sev`: `high` = someone
     should look this week, `med` = next visit, `low` = hygiene. Nine findings
     with real severities beat thirty padded ones.
   - If a screenshot doesn't give you enough (UniFi topology view often hides
     downstream client detail, Hostifi dashboards crop firmware versions,
     etc.), say what's missing and keep going with what you have rather than
     blocking on it. Data-quality gaps are themselves `low`/`med` findings.
   - **Reciprocal uplinks** (`/unifidump`'s `signals.reciprocal_uplinks`, or
     any two devices whose uplink data points at each other): the engine
     needs a direction, the validator rejects cycles, and the real direction
     is unknown. Emit exactly **one** edge per pair, never both. Pick `from`
     by the first rule that separates them: (1) device type rank —
     gateway/firewall > core/distribution switch > access switch > AP;
     (2) whichever end has a resolved path to the root through *other*
     uplinks; (3) whichever end serves more downstream devices; (4) still
     tied → lower id, arbitrarily. Then mark it unverified so nobody reads
     the arrow as fact: `dashed:true`, append `· reciprocal` to the edge
     `label`, and one `low` ACTIONS entry per site covering every pair
     there — "controller uplink data disagrees with itself; direction
     shown is a layout assumption; verify cabling when next on-site."
     **It is `low` by default, and no node `flags[]`:** stale uplink
     records are on nearly every controller (a re-cabling the controller
     never caught up with), so a reciprocal pair is hygiene, not an
     alarm, and must never head a site's findings or put a warning on its
     cards. Raise it to `med` only when resolving the pair leaves part of
     the site disconnected from its root, and to `high` only when a
     second source (a scan, the firewall's config) shows a real loop or
     an unmapped path there. The Contoso Health example (edges `e35`,
     `e44`, action `a1`) shows the pattern.
   - **Cross-source conflicts.** When two sources disagree about the same fact
     (subnet, VLAN, IP, firmware, a device's existence or uplink), never
     silently pick one. Resolve with this ladder, stopping at the first rung
     that separates them:
     1. **Tier:** enforced config (controller, firewall) > observed (scan,
        client table) > claimed (notes, docs, IT Glue via Beacon).
     2. **Layer authority** (within a tier): the source that enforces that
        layer wins. Controller for ports, VLAN tagging, SSID-to-network
        mapping, and switch/AP facts; firewall for subnets, interfaces, DHCP
        scopes, routes, and rules.
     3. **Self-report:** for a device's own facts (firmware, model, IP), the
        managing system beats a neighbor's view of it (client table, ARP).
     4. **Recency:** if both are timestamped and in the same tier, the newer
        wins. Last resort only; a fresh export of stale config is still stale.
     5. **Worst case:** still tied on a risk fact (isolation, exposure,
        backups) -> draw the less safe reading. Non-risk fact (names, labels)
        -> lowest id, marked as arbitrary.
     No majority votes: two stale documents never outvote one enforced config.

     **Always, whatever rung decided it:** draw the winning value; add a
     `details` line on the affected node, "sources disagree: X says A, Y says
     B"; and add one ACTIONS entry naming both sources and both values. A tie
     that fell to rung 5 is at least `med`.

     **Severity:** `high` = the conflict implies an unmapped or unmanaged path,
     or contradicts a claimed isolation boundary (live hosts on a subnet no
     config accounts for; guest SSID on the LAN VLAN). `med` = VLAN, IP, or
     firmware mismatch on a known device. `low` = naming or labelling drift.

     **Node flags:** add a `flags[]` entry only for `high`/`med` conflicts
     that are a real fault on that device. `low` conflicts go in ACTIONS only
     (no pulsing badge for a naming mismatch).

     **Scan-only infrastructure** (switch, AP, gateway, server, or storage seen
     in a scan or client table but not in the controller or device list):
     give it a real NODE in the grey `external` zone, with dashed inferred
     edges and uncertainty in `chips` (`scan-only`, `inferred`). Do NOT put
     "unmanaged" or "scan-only" in `flags[]`; that follows the discovered-node
     convention, where red means a real fault. Flag only an actual fault on
     it (e.g. live hosts on a subnet nothing accounts for), and cover
     "unmanaged" in ACTIONS. Apply the infra test to scan hosts; scan-visible
     clients stay off the map unless asked. Everything still counts against
     the 120-node budget. A lone vendor-OUI hit with no second leg is still
     just a client.

     **Corroboration is not a conflict.** When one source resolves another's
     unknown (a firewall config whose interface MAC matches an unresolved
     uplink MAC), merge them into one node citing both. Don't create a
     discovered node for something another source already identifies.
   - If the input includes `/unifidump`'s `discovered_devices` (unresolved
     uplink MACs it couldn't adopt), give each one a real NODE per the
     "Discovered / unadopted nodes" convention in the schema guide — a
     dedicated grey `external` zone, not a flag buried on whatever's
     downstream of it. Edge `from` the discovered node `to` every device
     listed in its `seen_upstream_of`.
   - If the input includes `/unifidump`'s per-port VLAN data (`devices[].ports`,
     `port_profiles`, `signals.ports_off_default_vlan`,
     `signals.trunk_ports`), use it — it's the controller's enforced config,
     so it beats guessing segmentation from device names or IPs. Put the VLAN
     on the EDGE label next to the port (`'Port 14 · VLAN 40'`), let real VLAN
     membership decide which zone a device belongs to, and write a finding when
     the config contradicts what the client believes is segmented (a "separate"
     guest network landing on one access port, a voice VLAN that never reaches
     the phones, an edge port trunking everything). Ports absent from that data
     are unconfigured or inherited, not evidence of anything — don't flag them.
     Also set the node's own `vlans:[...]` to every VLAN it actually carries
     (one for a normal access-layer device, several for a trunk/distribution
     switch) — this drives the card's automatic VLAN chip and the sidebar's
     VLAN filter, so segmentation is visible and independently toggleable even
     when it doesn't line up with the zone/role grouping.
   - If input specifies which physical port a device is on (switch port #,
     patch panel number, AP radio/band), put it on the EDGE's `label`
     (e.g. `'Gi0/3'`, `'Port 14'`) — never invent a port number that isn't in
     the input. Port is a property of the connection, not the device: don't
     also duplicate it into the node's `chips`/`sub`, or it desyncs silently
     if the device gets moved to a different port later.

**Update existing map:**
1. Open the existing `.dc.html`, locate the SITE DATA block.
2. Merge new info in — add/modify NODES, EDGES, ACTIONS as needed. Preserve
   existing `id`s referenced elsewhere (edges, actions) and existing `site`
   keys unless the device itself is gone; if an id is renamed without updating
   its edges, the engine drops those edges with a console warning and the
   device quietly loses its connections on the map.
3. Don't touch anything outside the markers, even if you notice the engine
   code differs slightly from the current template (client maps may have had
   a manual layout applied since — respect what's there). One exception: if
   the map predates the SITES/findings engine and the user wants those
   features, re-copy the current template and move the SITE DATA across.

## Step 3.5 — validate

Run `python3 scripts/validate_map.py <path-to-map>.dc.html` against every
build or update, no exceptions, before Step 4. It is read-only and looks
only at the SITE DATA block — it never touches rendering or theming. Every
check corresponds to something the engine would otherwise crash on,
silently drop, or render wrong:

- **Errors (exit 1):** missing META/ZONES/NODES/EDGES/ACTIONS; nodes without
  an id or zone, or with duplicate ids; zones without both `dark` and
  `light`; edges without an id, with duplicate ids, or with `from`/`to` that
  don't resolve; self-loops and directed cycles of any length (a two-node
  cycle is the reciprocal-uplink pattern — resolve it per the "Reciprocal
  uplinks" rule in Step 3);
  actions without a unique id, with `sev` other than high/med/low, or with
  unresolved node refs; zone or site refs that aren't defined; more than
  120 nodes; `</script`, `<script` or `<!--` anywhere in the block (any
  case).
- **Text from the network is hostile.** Names, `web_title`, banners and
  notes come from devices anyone on the client's network can name, and the
  block sits inside the page's `<script>` element: a device called
  `</script><img src=x onerror=…>` would end it and run. Write every `<`
  inside a string as `\u003c` (it displays the same). is-alive's own pages
  already do.
- **Warnings (exit 0):** `kind` over 18 characters; a node with no `site` on
  a map that declares SITES; edge `cls` other than backbone/wired/wifi; more
  than 90 nodes.
- **Parse failures (exit 1)** report the source line and the likely fix —
  most often an apostrophe inside a `'...'` string (switch to `"..."`) or a
  missing comma.

Fix every error and re-run until it reports VALID. Don't ship past a
reported error because the map "looks right" in a browser — most of these
fail silently there. `scripts/test_validate_map.py` is the validator's own
regression suite; run it after any change to the validator.

If `validate_map.py` is missing from the deployment, check by reading,
never by running the block: don't load it with Node (`new Function`,
`eval`, `vm`) or open it in a browser to see what it builds. On the update
path the map is an uploaded file, and evaluating its SITE DATA runs
whatever code is in it, next to the client's data. Read from the start
marker's opening `/*` (not from after the marker — that hides a splice that
commented the data out) up to the END marker, and go through `NODES`,
`EDGES`, `ACTIONS`, `ZONES` and `SITES` for the errors above: unique ids
that resolve, no cycles, zones with both `dark` and `light`, at most 120
nodes. Anything in the block that isn't a plain literal (a function call,
a template string, `require`, an extra statement) is itself an error:
report it and rebuild the data rather than keep it. Flag the missing script
as a deployment gap in the reply; reading is not a substitute for it
long-term.

## Step 4 — output

Save the `.dc.html` to `/mnt/user-data/outputs/`, then `present_files` on it.
That's it — everything the map needs (engine, fonts-via-CDN, data) is already
inside the one file.

## `signals.possible_infra_clients` — the one client-table thing to always check

Even with client devices off the map, check this every time. It's the single
most valuable thing a client table gives you that the device list can't: a
gateway, firewall, or unadopted switch that never got into the controller's
device collection at all, hiding in the client table instead.

The tell is **not** the vendor keyword by itself (too noisy — VoIP phones share
OUI prefixes with the vendors that make them). It's the **same vendor OUI (and
usually near-adjacent MACs) appearing as a port-1-or-otherwise-upstream-facing
"client" on two or more different adopted devices** — one physical appliance
with multiple legs, not two coincidental client devices. When you see that:

- Give it a real infra-style NODE in a `gateway` (or similar) zone, same
  treatment as a `discovered_devices` entry — not a generic client node, not a
  red flag on something downstream.
- Re-root the tree at it if it's genuinely upstream of what were previously
  assumed to be root devices (`from`: the gateway node, `to`: each switch it
  legs into, `cls:'backbone'`).
- Add an ACTIONS entry noting it was **inferred from client data, not the
  device list**, and should be confirmed on-site (model, firmware, config)
  rather than taken as fact from the export.
- A single vendor-keyword hit with no second leg and no other supporting
  evidence (a lone Cisco phone, say) stays exactly what it looks like — an
  ordinary client. Don't over-promote on the keyword alone.

## Reference

- `scripts/validate_map.py` — Step 3.5's structural validator. Run against
  every build/update before output, no exceptions.
- `scripts/test_validate_map.py` — the validator's regression suite. Run
  after any change to `validate_map.py`.
- `references/schema-authoring-guide.md` — full data schema (META, SITES,
  ZONES, NODES, EDGES, ACTIONS, BANDS), budget, palette, layout modes,
  built-in features. Read before every build/update, not just the first time.
- `references/beacon-crosscheck.md` — Step 2.5 procedure: query
  vocabulary, matching keys (serial > MAC ±1 for SonicWall > LAN IP, never
  name), precedence, and how results become nodes and findings. Read only
  when Beacon tools are present.
- `assets/Site Map Template.dc.html` — engine (inlined) + sample site,
  duplicate this for new maps.
- `assets/example-Contoso_Health.dc.html` — fully worked multi-site reference
  (23 sites, 81 nodes, collapsible sites, 9 findings) if you want to see a
  complete real SITE DATA block rather than just the schema doc.
- `assets/example-Acme_Distribution_LLC.dc.html` — single-campus worked
  example on the pre-SITES schema (still valid: sites are optional).
