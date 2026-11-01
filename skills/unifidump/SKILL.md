---
name: "unifidump"
description: "Extracts a neutral device/network/WLAN/port-VLAN summary from a UniFi controller export: a mongoexport-style JSON dump (flat array with __cmd/select/collection markers) or a native autobackup (.unf, or a .zip wrapping one, e.g. network_site_SITENAME_vVERSION_TIMESTAMP_unf.zip, containing db.gz). Trigger whenever the user uploads or references such a file, including for a site audit, firmware check, or topology troubleshooting, even if they do not name the format. Tells: JSON arrays with port_table, last_uplink, wlanconf_id, networkconf_id, adopted, or __cmd/collection markers; a .zip/.unf name containing unf; a zip holding db.gz. Stops at the summary and offers /mapnetwork; continues into a map only if the user sent /mapnetwork with the upload or accepts. Do NOT trigger for controller screenshots, exported CSVs, or nmap/scan output; /mapnetwork handles those directly."
---

# /unifidump

A raw UniFi controller mongo export is enormous and mostly noise for a
human (or for Claude) to read directly — thousands of historical client
records, a `rogue` collection full of every AP ever overheard in the
building, and device documents with 80+ fields most of which are
firmware internals nobody needs. But buried in there is exactly what
`/mapnetwork` needs: real devices, their physical uplink relationships,
and the VLAN/WLAN config. This skill's whole job is to pull that signal
out mechanically so the actual site-mapping work (in `/mapnetwork`)
starts from clean data instead of re-deriving it from scratch every time.

## Two input formats, same output

`extract.py` auto-detects and handles both, no flags needed:

- **mongoexport-style JSON** — a flat array mixing `{"__cmd":"select",
  "collection":"..."}` markers with plain-JSON documents. Produced by
  tooling that dumps the controller's mongo database to JSON.
- **Native `.unf` autobackup** (or a `.zip` wrapping one, e.g. the
  `network_site_<name>_v<ver>_<timestamp>_unf.zip` the controller UI
  produces) — a zip container holding `version`, `timestamp`, `db.gz`,
  and a `sites/` dir. `db.gz` is a gzip-compressed raw BSON document
  stream (the same wire format `mongodump` produces) — same
  `__cmd`/`select` marker structure underneath, just BSON-encoded
  instead of JSON-encoded. The script decodes this with a small
  dependency-free BSON reader built into `extract.py` itself — no
  `pymongo`, no pip install, no network access needed to run it.

A bare `db.gz` (if someone's already unzipped the container) and a `.zip`
with a nested `.unf` inside it (rather than `db.gz` at the top level) both
work too — detection recurses into nested zips looking for the BSON stream.

## Why this is a separate step from /mapnetwork

Reconstructing topology from a mongo dump is the same algorithm on every
site: split collections by the `__cmd` markers, build a MAC→name lookup,
and walk each device's `last_uplink.uplink_mac` to find what it's plugged
into. That's mechanical and deterministic — a script should do it, not
Claude reasoning through JSON (or BSON) by hand each time.

Deciding *what those facts mean* for a specific client — is a firmware
version actually a problem here, is a hidden SSID actually suspicious,
does this switch look orphaned — is a judgment call that depends on the
rest of the site's context. That reasoning belongs in `/mapnetwork`,
applied fresh each time, not pre-baked into this extraction step.

## What to do

1. Run the bundled script against the uploaded file — same command
   regardless of which input format it turns out to be:
   ```
   python3 scripts/extract.py <path-to-export> <output-path>.json
   ```
   `<path-to-export>` can be the `.json` dump, the `.unf`/`.zip`
   autobackup, or a bare `db.gz` — the script sniffs the file (zip magic
   bytes, then gzip magic bytes, then falls back to JSON) and prints
   which path it took to stderr. The script handles arbitrarily large
   exports (Fabrikam Care's test case had ~3900 top-level records,
   including 3000 rogue-AP entries and 800+ historical clients) and
   won't choke on the noise collections — it only touches `site`,
   `device`, `networkconf`, `portconf`, `wlanconf`, and `user` (the client
   table, for `clients` and `signals.possible_infra_clients`).

2. Read the resulting JSON. Its shape is intentionally neutral — no
   `zone`, no `flags[]`, no map-schema vocabulary:
   ```json
   {
     "site": { "id": "...", "name": "..." },
     "sites": [ { "id": "...", "name": "..." } ],
     "devices": [
       { "id": "<mac>", "name": "...", "type": "uap|usw|uacc|...",
         "model": "...", "mac": "...", "ip": "...", "version": "...",
         "adopted": true, "disabled": false, "port_count": 24,
         "site_id": "...", "site_name": "...or null if unresolved",
         "uplink": { "mac": "...", "name": "...or null if unresolved",
                     "remote_port": 5, "local_port": 1, "kind": "wire" },
         "ports": [ { "idx": 12, "name": "Nurse station", "profile": "Voice",
                      "forward": "native", "native_vlan": 40,
                      "native_network": "Voice", "tagged_vlans": [],
                      "poe_mode": "auto", "source": "port_overrides" } ] }
     ],
     "networks": [ { "name", "purpose", "vlan", "vlan_enabled", "subnet" } ],
     "port_profiles": [ { "id", "name", "forward", "native_vlan",
                          "native_network", "tagged_vlans", "voice_vlan",
                          "poe_mode", "isolation" } ],
     "wlans": [ { "name", "enabled", "security", "hidden", "vlan", "networkconf_name" } ],
     "signals": {
       "unresolved_uplinks": [ /* uplink_mac has no matching device */ ],
       "reciprocal_uplinks": [ /* two devices each claim the other as their uplink */ ],
       "firmware_versions": { "<version>": ["device names on it"] },
       "disabled_devices": [ /* devices with disabled:true, not just the key present */ ],
       "no_uplink_devices": [ /* no last_uplink recorded at all */ ],
       "ports_off_default_vlan": [ /* ports pinned to a real VLAN number */ ],
       "trunk_ports": [ /* ports carrying tagged VLANs */ ],
       "possible_infra_clients": [
         { "name", "mac", "oui", "uplink_name", "uplink_port" }
         /* clients whose OUI names a network/security vendor — a lead, not a verdict */ ]
     },
     "clients": [
       { "mac", "name", "hostname", "is_wired", "is_guest", "oui",
         "possible_infra_vendor", "ip",
         "uplink": { "mac", "name", "port" /* wired */ , "radio" /* wireless */ } /* or null */,
         "last_seen", "noted", "blocked", "site_id" } ],
     "discovered_devices": [
       { "mac": "e4:38:2f:00:00:01", "vendor": "Fortinet" /* or null */,
         "seen_upstream_of": ["Echo Clinic"], "source": "unifi_uplink" }
     ],
     "counts": { "<collection>": <doc count> }
   }
   ```
   `site` is the first site found, kept for the common single-site case.
   `sites` is every site in the export — **always check it**, since devices
   from a second or third site are still included but won't obviously belong
   together unless you look at `site_id`/`site_name` per device. A
   multi-site controller is normal for an MSP tenant (one export can carry
   20+ clinics); each becomes a `SITES` entry in the map, so carry
   `site_name` through per device rather than flattening it away. Devices
   whose `site_name` is null, and unresolved upstream MACs, belong in an
   "upstream / unmanaged" grouping rather than being assigned to a location
   they may not be at.

3. **Stop and report, then offer the map.** Give a short summary — site
   count, device count by type, client count, and any non-empty `signals`
   worth a look — and end with one line offering `/mapnetwork`. Don't start
   building. An export uploaded for a firmware check or an audit question
   gets its answer, not a map. Continue straight into a build only when the
   user sent `/mapnetwork` with the upload or says yes to the offer.

   When a build does happen, hand this off into `/mapnetwork`'s normal flow as the "device list"
   input — treat it the same as if the user had pasted a device
   spreadsheet or scan output. `/mapnetwork` still does all the
   zone-assignment, EDGE-building, and ACTIONS/flags reasoning; this
   skill only got it clean ingredients to work with.

   **What to pass, and what not to:**
   - `devices` — always. This is the map.
   - `discovered_devices` — always. `/mapnetwork` turns each entry into a
     discovered-style node wired to whatever it's `seen_upstream_of`, per its
     own schema guide.
   - `sites` / each device's `site_name` — always, and say how many there
     are. In `/mapnetwork` these become `SITES`: one file, one collapsible
     container per location. Never zones, never one file per site.
   - `signals.possible_infra_clients` — always, as a lead to check (see
     below). This is the one part of the client table that earns a place on
     the map.
   - `clients` — **not by default.** Report the count (`counts.user`, and a
     per-uplink tally if it's useful: "412 clients, 38 behind November-SW1") and
     leave them off the map. A 1,400-client export mapped device-by-device
     produces a file nobody can use. Pass the array through only when the
     user explicitly asks for client devices, and then only for the scope
     they asked about — `/mapnetwork` caps and collapses from there.

## Reading `ports` and `port_profiles`

Per-port VLAN is real config, not inference. UniFi keeps it in two places and
`extract.py` joins them: `portconf` is the site's switch-port profile table
(each profile naming a native network, an optional tagged set, a voice VLAN,
PoE mode), and each device's `port_overrides` says which profile is pinned to
which port. VLANs are resolved through `networkconf` so you get the actual
VLAN number and network name, not an opaque id.

- **`port_profiles`** — the site's profile table, once. `forward` is UniFi's
  own word for the port mode: `native` (plain access port), `customize`
  (native + an explicit tagged set — a trunk), `all` (every VLAN tagged),
  `disabled`.
- **`devices[].ports`** — only ports we know something real about, not a row
  per physical port: a 48-port switch with two configured ports emits two
  entries. `source` is `port_overrides` for an explicit admin override or
  `port_table` for a profile the port inherited.
- **`signals.ports_off_default_vlan`** — ports sitting on a numbered VLAN
  rather than the untagged default. This is the segmentation evidence: it's how
  you know the Voice VLAN actually reaches the phones, or that a "segmented"
  guest network terminates on exactly one port.
- **`signals.trunk_ports`** — ports carrying tagged VLANs. Normal between
  switches; worth a look on an edge port.

None of this is a verdict. A trunk to a closet switch and an access port on the
server VLAN are both entirely ordinary. What `/mapnetwork` does with it: VLAN
numbers on an edge label, a device's real VLAN membership driving which zone it
belongs to, and a finding when the config contradicts what the client believes
is segmented. Prefer this over guessing a device's VLAN from its name or IP —
this is what the controller is actually enforcing.

## Reading the signals section

These are facts, not verdicts — `/mapnetwork` (or you, reading this
directly) decides what's worth flagging on a given site:

- **`reciprocal_uplinks`** — device A's `last_uplink` points at device B,
  and B's points right back at A. Physically impossible (one end has to
  be upstream), so this means the controller's uplink data disagrees
  with itself — usually stale state from a re-cabling. Worth a physical
  verification, not an assumption either way.
- **`unresolved_uplinks`** — a device's `uplink_mac` doesn't match any
  device in this export. Either hardware that was never adopted into
  this controller, or something removed before the backup was taken.
- **`no_uplink_devices`** — no `last_uplink` recorded at all. Could mean
  offline, freshly adopted and never reconnected, or genuinely the root
  device (rare for anything but the gateway itself, which usually isn't
  in the `device` collection anyway — UniFi gateways/USGs are typically
  a separate adopted device your export may or may not include).
- **`firmware_versions`** — grouped so outliers are obvious at a glance.
  A site-wide baseline with one or two stragglers is a very different
  story than a site with no baseline at all.
- **`disabled_devices`** — only devices where `disabled` is actually
  `true`. Don't confuse this with a device merely *having* a `disabled`
  key in its document — many UniFi device records include the key set to
  `false`.

## Reading `discovered_devices`

This is a reshape of `signals.unresolved_uplinks`, not new data collection —
every distinct `uplink_mac` this controller never adopted, deduplicated
across every adopted device that reported it upstream, plus a best-effort
vendor guess from the MAC's OUI. Handed to `/mapnetwork` alongside `devices`,
this is what lets an unadopted firewall/switch/AP show up as an actual node
("UNKNOWN NETWORK DEVICE" or, with a vendor hit, "FORTINET DEVICE — MAC
only") instead of only a text flag on whatever it's upstream of.

`vendor` is `null`, never a guess, when the OUI isn't in `extract.py`'s small
local lookup table — that table is hand-assembled and **not verified against
the IEEE OUI registry**, so treat any vendor it returns as a lead worth
confirming on-site (or against `arp-scan`'s bundled OUI database), not as
fact to put in front of a client unqualified. Don't extend this into a model
or role guess ("Fortinet" ≠ "FortiGate 60F", "seen upstream of a switch" ≠
"is a firewall") — that inference belongs to `/mapnetwork`, made per-site
against whatever other evidence exists there.

## Reading `clients`

The `user` collection is UniFi's client/station table — every device that's
ever associated, wired or wireless. `extract_clients()` reshapes it into a
neutral `clients` array (`mac`, `name`/`hostname`, `is_wired`, `is_guest`,
`oui`, `ip`, `uplink`, `last_seen`, `noted`, `blocked`).

**This array is for analysis, not for the map.** Three things it's genuinely
good for:

1. **Finding hidden infrastructure** — the `possible_infra_clients` pattern
   below. Highest-value use, every time.
2. **Counting** — "412 named clients, 38 behind November-SW1, 0 on the guest
   SSID" is a useful line in the map's subtitle or a finding. Individual
   cards for all 412 are not.
3. **Data-quality findings** — clients with `uplink: null`, or a
   `last_seen` from two years ago, say something real about the controller's
   state. Surface as a `/mapnetwork` ACTIONS note.

Only hand the raw array to `/mapnetwork` when the user has asked for client
devices on the map.

`uplink` carries the correlation that makes this useful at all:
- **Wired clients**: `uplink.mac`/`uplink.name` (the switch) plus
  `uplink.port` (the actual switch port) — from `last_uplink_mac`,
  `last_uplink_name`, `last_uplink_remote_port` on the doc.
- **Wireless clients**: `uplink.mac`/`uplink.name` (the AP) plus
  `uplink.radio` (band) — no port, WiFi doesn't have one.
- A handful of records (typically stale/long-disconnected — check
  `last_seen`) have none of these fields at all. Those come through with
  `uplink: null` rather than being silently dropped; nothing can place them
  on a map (no known upstream), and nothing should fabricate a connection
  for them — count them and surface as a data-quality note.

`oui` here is UniFi's own client-side vendor string — a **different source**
than `discovered_devices`'s local OUI-to-vendor table above. Don't merge or
cross-check the two against each other; they're independent guesses from
independent code, and treating one as validating the other would be false
confidence.

## Reading `signals.possible_infra_clients`

A client's `oui` occasionally names a networking/security vendor
(SonicWall, Cisco, Fortinet, MikroTik, Aruba, Juniper, Ubiquiti, and a
handful of others — see `_INFRA_VENDOR_KEYWORDS` in `extract.py`) even
though UniFi only ever sees it as a generic client. That's usually because
it's actual infrastructure — a gateway, firewall, or switch nobody adopted
into this controller — showing up as a plain client because nothing in the
`user` collection distinguishes "end-user device" from "unmanaged network
gear." This signal is a **keyword match only, not a verdict**: a Cisco IP
phone (`SEP<mac>` hostnames are the tell) has a Cisco OUI too and is a
completely ordinary VoIP endpoint, not infrastructure. Expect noise.

What's actually worth `/mapnetwork` pulling out of the generic client pool
into its own real node (same treatment as a `discovered_devices` entry,
not a red flag) is a *pattern*, not just the keyword hit alone — most
tellingly: **the same OUI vendor appearing on port 1 (or otherwise clearly
upstream-facing) of two or more different adopted devices**, which usually
means one physical appliance with multiple legs into the network, not two
unrelated client devices. A single Cisco phone hit is noise; two SonicWall
MACs on port 1 of two different core switches is a gateway.

## A note on scale

Don't try to read the raw dump file directly with `view` or `cat` before
running the script. For JSON exports the signal-to-noise ratio is bad
(the `rogue` collection alone can be thousands of entries of overheard
neighboring APs, and `user` holds every client that's ever connected,
often 500+ entries). For `.unf`/`.zip` exports it's binary (a zip
container around gzip-compressed BSON) — `view`/`cat` will just show you
garbage, not a hint that something's wrong with the file. Run the script
first; read its output.
