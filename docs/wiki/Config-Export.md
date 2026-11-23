Decodes a UniFi backup (`.unf` / `.unifi`), SonicWall settings export (`.exp`) or tech support report (`.wri`), or Meraki MX config (Dashboard API `.json`) on this machine and saves sanitized JSON for the unifidump / mapnetwork skills; also reads a `/mapnetwork` map (`.dc.html`, or a zip holding one) back in for the Map tab, and hands a saved map (`.json`) to the Map. UniFi backups, SonicWall exports and `/mapnetwork` maps draw the [[Map]]; **Meraki configs go into the Claude Handoff and the JSON for Claude, but aren't drawn on the Map** (the sub-tab says so).

Open files with **Open file…**, from the Map (**More › Open…**, the same dialog; you stay on the Map), by dropping them on the window, or by naming them on the command line (**Open with → is-alive**). Opening from here keeps you here; a saved map opens on the Map.

- **Drops** go where they make sense on the tab they land on, and the overlay says where: on the Map, Config Export reads them and the Map stays on screen and redraws; on Notes, a photo or notes (`.md`, `.txt`) go into Notes and anything else comes here; on any other tab this tab comes on screen to read them. A file that can't be read while you're on another tab says why in a message at the bottom of the window.
- **While a file is being read**, files opened or dropped (and example maps picked) wait their turn and are read right after it; nothing is thrown away.
- **Open with** (or the command line) reads the open site's own configs first, then the files named, and the site remembers them all.
- **A Claude Handoff zip** isn't read back: it says *that's a Claude Handoff zip* and to open the backups and exports it was made from, or the map Claude made from it. A zip holding Claude's `.dc.html` opens as that map.

## UniFi

**Open file…** takes a `.unf` (site export or classic autobackup), a `.unifi` (UniFi OS console backup), a `.zip` holding one, or a bare `db.gz`; the format is detected from the content, not the name. Everything runs locally: nothing is uploaded or sent anywhere.

| File | Decrypt | Inside |
|---|---|---|
| `.unf` | AES-128-CBC, UniFi's static key | zip → `db.gz` |
| `.unifi` | AES-256-CBC, UniFi's static key, IV in the first 16 bytes | gzip → tar → `backup/network/db.gz` |

**Save JSON…** writes `<name>.unifidump.json`: the mongoexport-style array (`{"__cmd":"select","collection":…}` markers) that the unifidump skill's `extract.py` reads directly, so hand that file to `/unifidump` instead of the backup.

- **Kept:** `site`, `device`, `networkconf`, `portconf`, `wlanconf`, and `user` (clients) only when **Client names** is *anonymized* or *full* (default *none* here; the Claude Handoff defaults to *anonymized*). Client hostnames are often people's names; unifidump uses them only for counts and to spot unadopted gear plugged in as a client. Client notes, email addresses and phone numbers are dropped in every mode.
- **Left behind:** every other collection: admins and their password hashes, settings, events, alarms, stats, rogue APs, and so on. The console's own config, TLS keys and Postgres database in a `.unifi` are never read.
- **Removed from what's kept:** any field starting `x_` (UniFi's prefix for stored secrets: WLAN passphrases, device auth keys, WAN/VPN passwords) or named like a password, passphrase, secret, PSK, private key, token, certificate, hash or user name (a PPPoE login is the ISP account's ID), at any depth. Also any value that holds key material, whatever its field is called and in any case: a PEM certificate or key, an OpenVPN config's inline keys, a WireGuard config's `PrivateKey` or `PresharedKey` (as text or base64, so a config uploaded under a field newer firmware adds is caught too), or a bare 32-byte key in base64. Lists are checked item by item, and a removed item is counted as `field[]`.

The summary lists the layers peeled, the collections kept and left behind with counts, and every field removed, so you can check what goes out before sharing it. Names, MACs, IPs, VLANs and topology stay: that is what unifidump maps.

## SonicWall

Pick several files at once, or drag them onto the window, at once or one after another. Each file gets its own sub-tab (*kind · name*) with **Save JSON…**, **Copy** and **Clear**; the file opened last is shown, and opening the same file again replaces its tab. Past six tabs the oldest fold into a **n more** menu. The Map and the Claude Handoff use every UniFi backup and SonicWall export open (the Claude Handoff every Meraki export too), and the `/mapnetwork` map opened last; Notes redacts the secrets of every open file. Changing **Client names** reads every open UniFi backup again and changes what a Meraki export's **Save JSON…** writes.

**Several sites or firewalls:** open all of a visit's files, even several of one kind (a client with two sites, each with its own UniFi backup and SonicWall report, is four files):

- **UniFi backups** all go on the [[Map]]. A device or client in two backups (one controller backed up on two days) is drawn once, as the backup opened later has it. Each controller's sites keep their own box; two sites of one name (both *Default*) take the part of their backup's file name that differs: *Default (north)*, *Default (south)*.
- **SonicWall exports**: each firewall is a gateway of its own on the Map. A settings export and a tech support report of the same firewall are one card.
- In the [[Claude Handoff|Site-Bundle-and-Claude]] each file goes in on its own: `unifi.unifidump.json`, `unifi-2.unifidump.json`, `sonicwall.json`, `sonicwall-2.json`, `meraki-2.json` and so on.

The same **Open file…** takes a SonicWall settings export (`.exp`, Gen 6 or 7) or one already decoded to text. The file is only base64 and URL-encoding; it's decoded here and turned into JSON for `/mapnetwork`: interfaces (zone, physical/VLAN with tag and parent, IP/mask, gateway, secondary subnet, port MAC, which management protocols are on, comment), zones, address objects and groups, FQDN objects, service objects and groups, access rules (zones, source, destination, service, action, enabled, comment) and NAT rules.

It also takes a **tech support report** (`.wri`, the file SonicOS saves from its Tech Support Report; tested on SonicOS 7.3). A TSR is a readable report rather than settings, so it has its own reader that gives the same JSON: model, serial and firmware; interfaces (zone, VLAN and parent, IPv4 address and mask, WAN gateway, port MAC, IPv4 management protocols, shut-down ports, comment); zones and their security type; address objects (host, network, range, FQDN) and groups; access rules (zones, source, destination, services, allow/deny/discard, enabled, comment) and NAT policies. It also reads the **devices the firewall heard on each port** (*Advanced Switching → L2 Discovery*): an LLDP neighbour (switch, AP, phone) is plugged into that port, so it goes on the Map under the firewall with a real link labelled with both ports ("X0, its Port 24"), and a UniFi device whose uplink was only inferred gets that real link. LLTD neighbours are Windows hosts: they go on as clients with an assumed link, since LLTD crosses switches, and without their computer names, which are often people's names. An LLDP neighbour whose capabilities say it's only a station (a PC running an LLDP agent) loses its name and description the same way. The report's ARP cache and DHCP bindings are options when it's downloaded and are off by default; they aren't read yet. Only those sections are read, field by field; users, VPN, RADIUS/LDAP, SNMP, certificates, licences, registration codes, logs and diagnostics are skipped whole. Service objects aren't read from a report (rules name their services).

The output is built only from those named fields, so admin and user accounts, passwords, VPN pre-shared keys, RADIUS/LDAP secrets, SNMP communities, certificates and PPPoE logins never leave the file; the summary says how many settings were copied and how many left behind. Comments are kept because they say what a rule or port is for; any that mention a password, PSK, secret, community or token are replaced with a placeholder, and the summary counts them. Interface MACs are what `/mapnetwork` matches against `/unifidump`'s unresolved uplink MACs to name the firewall.

## Meraki MX

Meraki keeps no config file on the box: its config is what the Dashboard API returns. [`scripts/meraki-export.ps1`](https://github.com/hammerknife404/is-alive/blob/main/scripts/meraki-export.ps1) saves an organization's into one JSON file (read-only GET calls; the API key comes from `MERAKI_DASHBOARD_API_KEY` or a hidden prompt and is never written):

```powershell
.\meraki-export.ps1 -Org "Harbor Dental" -Out harbor.meraki.json
```

Open that file (or a zip of one JSON file per API call, as other backup scripts save it) in Config Export. Tables are recognised by their shape, wherever they sit: organization, networks, devices (MX models, serials and firmware go in the summary), VLANs with their DHCP reservations (or the single LAN), appliance ports, L3 / inbound / cellular firewall rules in order, L7 rules, port forwarding, 1:1 and 1:many NAT, static routes, site-to-site VPN mode, hubs and subnets, third-party VPN peers, WAN uplink settings and status, and MX Wi-Fi SSIDs. A file in a zip folder named after a network is tagged with that network. Pre-shared keys, VPN, RADIUS and SNMP secrets and PPPoE passwords are never copied (Notes redacts them, as with the other configs); rule comments and names that mention a password or key are withheld, and DHCP reservation names and reserved ranges' comments are cleaned the same way (a password, email address or phone number in one is withheld). **Client names** decides what a reservation for a phone, laptop or PC (anything not recognised by its MAC's maker as network gear, a NAS, a printer or a camera) is called: no name (*none*, the default here), a `client-xxxxxxxx` token made from its MAC, the same one UniFi and the sweep inventory use (*anonymized*), or its own (*full*). Other reservations keep their names. The raw export holds those secrets and names, so share the converted JSON, not the export.

The Map doesn't draw Meraki configs: they go into the [[Claude Handoff|Site-Bundle-and-Claude]] (`meraki.json`, `meraki-2.json`, …) and **Save JSON…**, for `/mapnetwork` in Claude. Opened from the Map, a Meraki config is read here all the same, and a message says the Map doesn't draw it.

## /mapnetwork maps

A `/mapnetwork` map (`.dc.html`, or a zip holding one, such as a handoff zip you saved it into) opens here too and becomes the [[Map]]. See [[Claude Handoff|Site-Bundle-and-Claude]] for the round trip.

## Saved maps

A saved map's `.json` (the Map's **Export JSON…**, yours or a colleague's) opened here goes to the [[Map]] as a saved map, and the Map comes on screen. It keeps the date it was saved, so **Compare** doesn't take a colleague's old map for today's visit.
