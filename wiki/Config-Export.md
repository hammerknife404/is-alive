## UniFi

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

## SonicWall

Pick several files at once, or drag them onto the window from any tab, at once or one after another. Each file gets its own sub-tab (*kind · name*) with **Save JSON…**, **Copy** and **Clear**; the file opened last is shown, and opening the same file again replaces its tab. Past six tabs the oldest fold into a **n more** menu. The Map, Site Bundle and Notes use the file of each kind opened last; Notes redacts the secrets of every open file.

The same **Open file…** takes a SonicWall settings export (`.exp`, Gen 6 or 7) or one already decoded to text. The file is only base64 and URL-encoding; it's decoded here and turned into JSON for `/mapnetwork`: interfaces (zone, physical/VLAN with tag and parent, IP/mask, gateway, secondary subnet, port MAC, which management protocols are on, comment), zones, address objects and groups, FQDN objects, service objects and groups, access rules (zones, source, destination, service, action, enabled, comment) and NAT rules.

It also takes a **tech support report** (`.wri`, the file SonicOS saves from its Tech Support Report; tested on SonicOS 7.3). A TSR is a readable report rather than settings, so it has its own reader that gives the same JSON: model, serial and firmware; interfaces (zone, VLAN and parent, IPv4 address and mask, WAN gateway, port MAC, IPv4 management protocols, shut-down ports, comment); zones and their security type; address objects (host, network, range, FQDN) and groups; access rules (zones, source, destination, services, allow/deny/discard, enabled, comment) and NAT policies. It also reads the **devices the firewall heard on each port** (*Advanced Switching → L2 Discovery*): an LLDP neighbour (switch, AP, phone) is plugged into that port, so it goes on the Map under the firewall with a real link labelled with both ports ("X0, its Port 24"), and a UniFi device whose uplink was only assumed gets that real link. LLTD neighbours are Windows hosts: they go on as clients with an assumed link, since LLTD crosses switches, and without their computer names, which are often people's names. The report's ARP cache and DHCP bindings are options when it's downloaded and are off by default; they aren't read yet. Only those sections are read, field by field; users, VPN, RADIUS/LDAP, SNMP, certificates, licences, registration codes, logs and diagnostics are skipped whole. Service objects aren't read from a report (rules name their services).

The output is built only from those named fields, so admin and user accounts, passwords, VPN pre-shared keys, RADIUS/LDAP secrets, SNMP communities, certificates and PPPoE logins never leave the file; the summary says how many settings were copied and how many left behind. Comments are kept because they say what a rule or port is for; any that mention a password, PSK, secret, community or token are replaced with a placeholder, and the summary counts them. Interface MACs are what `/mapnetwork` matches against `/unifidump`'s unresolved uplink MACs to name the firewall.

## Meraki MX

Meraki keeps no config file on the box: its config is what the Dashboard API returns. [`scripts/meraki-export.ps1`](https://github.com/hammerknife404/is-alive/blob/main/scripts/meraki-export.ps1) saves an organization's into one JSON file (read-only GET calls; the API key comes from `MERAKI_DASHBOARD_API_KEY` or a hidden prompt and is never written):

```powershell
.\meraki-export.ps1 -Org "Harbor Dental" -Out harbor.meraki.json
```

Open that file (or a zip of one JSON file per API call, as other backup scripts save it) in Config Export. Tables are recognised by their shape, wherever they sit: organization, networks, devices (MX models, serials and firmware go in the summary), VLANs with their DHCP reservations (or the single LAN), appliance ports, L3 / inbound / cellular firewall rules in order, L7 rules, port forwarding, 1:1 and 1:many NAT, static routes, site-to-site VPN mode, hubs and subnets, third-party VPN peers, WAN uplink settings and status, and MX Wi-Fi SSIDs. A file in a zip folder named after a network is tagged with that network. Pre-shared keys, VPN, RADIUS and SNMP secrets and PPPoE passwords are never copied (Notes redacts them, as with the other configs); rule comments and names that mention a password or key are withheld. The raw export holds those secrets, so share the converted JSON, not the export.
