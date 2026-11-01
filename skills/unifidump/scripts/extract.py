#!/usr/bin/env python3
"""
Extract a clean, neutral intermediate representation from a raw UniFi
controller MongoDB export (the kind produced by mongoexport/mongodump-to-json
tooling, where the file is a flat JSON array mixing `{"__cmd":"select",
"collection":"..."}` markers with the documents that follow them).

Usage:
    python3 extract.py <input> [output.json]

<input> can be a mongoexport-style JSON dump, a raw db.gz, or a native
UniFi .unf autobackup / .zip wrapping one — format is auto-detected.
If output.json is omitted, writes to <input-stem>.extracted.json alongside
the input.

Output shape (neutral — no zones, no flags, no map-schema opinions):
{
  "site": {id, name},   # first site in the export; kept for back-compat
  "sites": [ {id, name}, ... ],  # every site found — check this for multi-site exports
  "devices": [ { id, name, type, model, mac, ip, version, adopted,
                 uplink: {mac, remote_port, kind} | null,
                 disabled, port_count, site_id, site_name,
                 ports: [ { idx, name, profile, forward, native_vlan,
                            native_network, tagged_vlans, poe_mode,
                            op_mode, source } ... ] } ... ],
  "networks": [ { name, purpose, vlan, subnet, vlan_enabled } ... ],
  "port_profiles": [ { id, name, forward, native_vlan, native_network,
                       tagged_vlans, poe_mode, isolation } ... ],
  "wlans": [ { name, enabled, security, hidden, networkconf_name } ... ],
  "signals": {
      "unresolved_uplinks": [ {device_name, device_mac, uplink_mac} ... ],
      "reciprocal_uplinks": [ {a_name, a_mac, b_name, b_mac} ... ],
      "firmware_versions": { version_string: [device_name, ...] },
      "disabled_devices": [device_name, ...],
      "no_uplink_devices": [device_name, ...],
      "clients_no_uplink": [client_name_or_mac, ...],
      "ports_off_default_vlan": [ {device_name, port_idx, port_name, profile,
                                   native_vlan, native_network, forward} ... ],
      "trunk_ports": [ {device_name, port_idx, profile, tagged_vlans} ... ],
      "possible_infra_clients": [ {name, mac, oui, uplink_name, uplink_port} ... ]
  },
  "discovered_devices": [
      { "mac": "...", "vendor": "Fortinet" | null,
        "seen_upstream_of": ["device_name", ...], "source": "unifi_uplink" } ... ],
  "clients": [
      { mac, name, hostname, is_wired, is_guest, oui, possible_infra_vendor,
        ip, uplink: {mac, name, port, radio} | null,
        last_seen, noted, blocked, site_id } ... ],
  "counts": { collection_name: count, ... }
}

`discovered_devices` is derived straight from `signals.unresolved_uplinks` — every
uplink_mac this controller never adopted, deduplicated across whichever adopted
devices reported it, with a best-effort vendor guess from the MAC's OUI (first 3
octets). This is *not* new data collection — it's the same unresolved-uplink fact
UniFi already recorded, reshaped so `/mapnetwork` can render it as a node instead
of only a text flag. `vendor` is null whenever the OUI isn't in the (small,
non-exhaustive) local lookup table — never guess a vendor from anything else, and
never infer model/role from vendor alone; that judgment call belongs to
`/mapnetwork`, made fresh per site.

Why this shape: it's deliberately free of any client-map vocabulary (no
"zone", no "flags[]" in the map-schema sense, no severity). Topology
reconstruction and naming resolution are mechanical and identical for every
site; deciding what's *risky* about a given topology is a judgment call that
belongs to whoever is building the actual client-facing map, made fresh
against whatever this site's normal looks like.
Input formats accepted (auto-detected, no flags needed):
  - mongoexport-style JSON (the original format — flat array of __cmd/doc dicts)
  - a raw `db.gz` file (gzip-compressed BSON document stream)
  - a native UniFi controller autobackup `.unf` file (a zip container)
  - a `.zip` wrapping that same `.unf` layout (version/timestamp/db.gz/sites/),
    e.g. what the controller UI names `network_site_<name>_v<ver>_<ts>_unf.zip`

The `.unf`/`.zip` container's `db.gz` is a raw BSON document stream (the same
wire format mongodump produces) — decoded here with a small dependency-free
BSON reader rather than pulling in pymongo, so this script has zero external
deps and doesn't need network access to run.
"""
import gzip
import io
import json
import struct
import sys
import zipfile
from collections import defaultdict


# ---------------------------------------------------------------------------
# Minimal dependency-free BSON reader.
#
# Only decodes what UniFi controller exports actually contain (documents,
# arrays, strings, ints, doubles, bools, ObjectId, UTC datetime, null,
# binary). Anything more exotic (regex, JS code, Decimal128, DBPointer) is
# decoded structurally so it doesn't crash, but represented loosely — none of
# it appears in `site`/`device`/`networkconf`/`wlanconf` documents in
# practice.
# ---------------------------------------------------------------------------

def _read_cstring(buf, pos):
    end = buf.index(b"\x00", pos)
    return buf[pos:end].decode("utf-8", "replace"), end + 1


def _read_element(buf, pos, type_byte):
    if type_byte == 0x01:  # double
        val = struct.unpack_from("<d", buf, pos)[0]
        return val, pos + 8
    if type_byte == 0x02:  # string
        length = struct.unpack_from("<i", buf, pos)[0]
        pos += 4
        val = buf[pos:pos + length - 1].decode("utf-8", "replace")
        return val, pos + length
    if type_byte == 0x03:  # embedded document
        length = struct.unpack_from("<i", buf, pos)[0]
        val = _decode_document(buf[pos:pos + length])
        return val, pos + length
    if type_byte == 0x04:  # array
        length = struct.unpack_from("<i", buf, pos)[0]
        doc = _decode_document(buf[pos:pos + length])
        # array is encoded as a document with keys "0","1",... — reorder by
        # numeric key so order is preserved even if a key is skipped.
        try:
            val = [doc[k] for k in sorted(doc, key=int)]
        except (ValueError, KeyError):
            val = list(doc.values())
        return val, pos + length
    if type_byte == 0x05:  # binary
        length = struct.unpack_from("<i", buf, pos)[0]
        subtype = buf[pos + 4]
        data = buf[pos + 5:pos + 5 + length]
        return {"$binary": data.hex(), "$subtype": subtype}, pos + 5 + length
    if type_byte == 0x06:  # undefined (deprecated)
        return None, pos
    if type_byte == 0x07:  # ObjectId
        return buf[pos:pos + 12].hex(), pos + 12
    if type_byte == 0x08:  # boolean
        return bool(buf[pos]), pos + 1
    if type_byte == 0x09:  # UTC datetime (ms since epoch)
        ms = struct.unpack_from("<q", buf, pos)[0]
        return ms, pos + 8
    if type_byte == 0x0A:  # null
        return None, pos
    if type_byte == 0x0B:  # regex — pattern cstring + options cstring
        pattern, pos = _read_cstring(buf, pos)
        options, pos = _read_cstring(buf, pos)
        return {"$regex": pattern, "$options": options}, pos
    if type_byte == 0x0C:  # DBPointer (deprecated) — string + 12-byte oid
        length = struct.unpack_from("<i", buf, pos)[0]
        pos += 4 + length
        return None, pos + 12
    if type_byte == 0x0D:  # JavaScript code
        length = struct.unpack_from("<i", buf, pos)[0]
        pos += 4
        val = buf[pos:pos + length - 1].decode("utf-8", "replace")
        return val, pos + length
    if type_byte == 0x0E:  # symbol (deprecated) — same wire shape as string
        length = struct.unpack_from("<i", buf, pos)[0]
        pos += 4
        val = buf[pos:pos + length - 1].decode("utf-8", "replace")
        return val, pos + length
    if type_byte == 0x0F:  # code with scope
        total_len = struct.unpack_from("<i", buf, pos)[0]
        return None, pos + total_len
    if type_byte == 0x10:  # int32
        val = struct.unpack_from("<i", buf, pos)[0]
        return val, pos + 4
    if type_byte == 0x11:  # Timestamp (8 bytes)
        val = struct.unpack_from("<Q", buf, pos)[0]
        return val, pos + 8
    if type_byte == 0x12:  # int64
        val = struct.unpack_from("<q", buf, pos)[0]
        return val, pos + 8
    if type_byte == 0x13:  # Decimal128 (16 bytes) — not used by UniFi docs
        return buf[pos:pos + 16].hex(), pos + 16
    if type_byte in (0xFF, 0x7F):  # MinKey / MaxKey
        return None, pos
    raise ValueError(f"Unsupported BSON type 0x{type_byte:02x} at offset {pos}")


def _decode_document(buf):
    # buf[0:4] is the document's own length prefix (already sliced to size
    # by the caller); start reading elements right after it.
    pos = 4
    end = len(buf) - 1  # trailing NUL terminator
    doc = {}
    while pos < end:
        type_byte = buf[pos]
        pos += 1
        key, pos = _read_cstring(buf, pos)
        value, pos = _read_element(buf, pos, type_byte)
        doc[key] = value
    return doc


def decode_bson_stream(data):
    """Decode a concatenated stream of top-level BSON documents (the format
    both mongodump and UniFi's own db.gz autobackup use) into a list of
    dicts."""
    docs = []
    pos = 0
    n = len(data)
    while pos < n:
        length = struct.unpack_from("<i", data, pos)[0]
        doc = _decode_document(data[pos:pos + length])
        docs.append(doc)
        pos += length
    return docs


# ---------------------------------------------------------------------------
# Input format detection — figures out what kind of file we were handed and
# gets it down to a flat list of dicts (the mongoexport-JSON shape) before
# handing off to split_collections(), which doesn't care which path got it
# there.
# ---------------------------------------------------------------------------

def _find_db_gz_bytes(zf):
    """Given an open ZipFile, find and decompress the db.gz entry. Handles
    the flat layout (db.gz at the top level) and the nested case where a
    .unf (itself a zip) is stored inside an outer .zip wrapper."""
    names = zf.namelist()
    gz_names = [n for n in names if n.endswith("db.gz")] or \
               [n for n in names if n.endswith(".gz")]
    if gz_names:
        with zf.open(gz_names[0]) as f:
            return gzip.decompress(f.read())

    # No db.gz at this level — look for a nested .unf (zip) and recurse.
    unf_names = [n for n in names if n.lower().endswith(".unf")]
    for name in unf_names:
        with zf.open(name) as f:
            inner_bytes = f.read()
        if zipfile.is_zipfile(io.BytesIO(inner_bytes)):
            with zipfile.ZipFile(io.BytesIO(inner_bytes)) as inner_zf:
                found = _find_db_gz_bytes(inner_zf)
                if found is not None:
                    return found
    return None


def load_export_documents(input_path):
    """Return a flat list of dicts (mongoexport-JSON shape) regardless of
    whether input_path is JSON, a raw db.gz, or a .unf/.zip container."""
    with open(input_path, "rb") as f:
        head = f.read(4)

    # zip container: native .unf (autobackup) or a .zip wrapping one.
    if zipfile.is_zipfile(input_path):
        with zipfile.ZipFile(input_path) as zf:
            bson_bytes = _find_db_gz_bytes(zf)
        if bson_bytes is None:
            raise ValueError(
                f"{input_path} is a zip but no db.gz (or nested .unf "
                "containing one) was found inside it"
            )
        print(f"  detected zip/.unf container, decoded db.gz "
              f"({len(bson_bytes)} bytes BSON)", file=sys.stderr)
        return decode_bson_stream(bson_bytes)

    # raw gzip: either a bare db.gz handed over on its own, or a .unf that
    # (unusually) wasn't a zip but plain gzip.
    if head[:2] == b"\x1f\x8b":
        with open(input_path, "rb") as f:
            bson_bytes = gzip.decompress(f.read())
        print(f"  detected raw gzip BSON stream ({len(bson_bytes)} bytes)",
              file=sys.stderr)
        return decode_bson_stream(bson_bytes)

    # otherwise assume mongoexport-style JSON (the original expected format)
    with open(input_path, "r", encoding="utf-8") as f:
        return json.load(f)


# ---------------------------------------------------------------------------
# Minimal OUI (first-3-octet) -> vendor lookup for discovered_devices' vendor
# guess. Deliberately small and static — no network lookup, no third-party
# OUI database bundled. This covers common network/security/infra vendors
# likely to show up as an unresolved uplink; anything not in here just gets
# vendor: null rather than a wrong guess. Extend as you actually encounter
# new OUIs in the field — don't try to make this exhaustive up front.
# ---------------------------------------------------------------------------
_OUI_VENDORS = {
    "04d6aa": "Fortinet", "00098b": "Fortinet", "0090fb": "Fortinet",
    "70728c": "Cisco", "001b0c": "Cisco", "0021d8": "Cisco", "6c9c42": "Cisco",
    "88157e": "Meraki (Cisco)", "0018ce": "Meraki (Cisco)",
    "00179a": "SonicWall", "0017c5": "SonicWall",
    "0009de": "Palo Alto Networks", "50609d": "Palo Alto Networks",
    "241b8f": "Palo Alto Networks",
    "d8b370": "Aruba (HPE)", "24de91": "Aruba (HPE)",
    "cc2d21": "MikroTik", "48a98a": "MikroTik", "6c3b6b": "MikroTik",
    "4c5e0c": "Ubiquiti", "24a43c": "Ubiquiti", "fcecda": "Ubiquiti",
    "dc9fdb": "Synology", "0011d9": "Synology",
    "b827eb": "Raspberry Pi Foundation", "dca632": "Raspberry Pi Foundation",
    "00408c": "Axis Communications", "accc8e": "Axis Communications",
    "9c8e99": "HP / Aruba networking", "3ca82a": "HP Inc.",
    "001a1e": "Dell", "b8ca3a": "Dell",
    "005056": "VMware",
    "0050f2": "Microsoft",
    "d8cb8a": "Apple", "3c0754": "Apple",
    "e4382f": "Fortinet",
    "845b12": "Netgear", "204e7f": "Netgear",
    "c46e1f": "TP-Link", "b0be76": "TP-Link",
    "509a4c": "Juniper Networks", "8c6266": "Juniper Networks",
}
# NOTE: this table was hand-assembled from memory, not cross-checked against
# the IEEE OUI registry. Treat every vendor string it returns as a *lead*,
# not ground truth — verify against https://standards-oui.ieee.org/ (or
# `arp-scan`'s bundled OUI database, which is authoritative and free) before
# trusting it on a real site. Wrong OUIs here are worse than no lookup at
# all, since they'd masquerade as fact in a client-facing map.


def _vendor_from_mac(mac):
    """Best-effort vendor from a MAC's OUI. Returns None (never a guess)
    when the prefix isn't in the local table."""
    if not mac:
        return None
    prefix = mac.replace(":", "").replace("-", "").lower()[:6]
    return _OUI_VENDORS.get(prefix)


def build_discovered_devices(unresolved_uplinks):
    """Turn signals.unresolved_uplinks into deduplicated discovered-device
    entries — one per unique uplink_mac, listing every adopted device that
    reported seeing it upstream. Purely a reshape of data extract_devices/
    compute_signals already produced; no new fields are invented beyond the
    OUI vendor guess."""
    by_mac = {}
    for u in unresolved_uplinks:
        entry = by_mac.setdefault(u["uplink_mac"], {
            "mac": u["uplink_mac"],
            "vendor": _vendor_from_mac(u["uplink_mac"]),
            "seen_upstream_of": [],
            "source": "unifi_uplink",
        })
        if u["device_name"] not in entry["seen_upstream_of"]:
            entry["seen_upstream_of"].append(u["device_name"])
    return list(by_mac.values())


def split_collections(data):
    """Split the flat mongo-export array into named collections using the
    __cmd/select markers that precede each collection's documents."""
    collections = defaultdict(list)
    current = None
    for item in data:
        if isinstance(item, dict) and item.get("__cmd") == "select":
            current = item.get("collection")
        elif current is not None:
            collections[current].append(item)
    return collections


def extract_devices(device_docs, site_name_by_id=None, port_profiles=None):
    site_name_by_id = site_name_by_id or {}
    profiles_by_id = {p["id"]: p for p in (port_profiles or [])}
    devices = []
    mac_to_name = {}
    for d in device_docs:
        name = d.get("name")
        mac = d.get("mac")
        if not name:
            # UniFi leaves `name` unset for some device types (e.g. mesh
            # access points / satellite hardware) — fall back to a
            # type+mac-suffix label so it's still identifiable downstream.
            name = f"{d.get('type', 'device')}-{mac[-5:] if mac else 'unknown'}"
        mac_to_name[mac] = name

    for d in device_docs:
        mac = d.get("mac")
        last_uplink = d.get("last_uplink") or None
        uplink = None
        if last_uplink and last_uplink.get("uplink_mac"):
            uplink = {
                "mac": last_uplink.get("uplink_mac"),
                "name": mac_to_name.get(last_uplink.get("uplink_mac")),  # None if unresolved
                "remote_port": last_uplink.get("uplink_remote_port"),
                "local_port": last_uplink.get("port_idx"),
                "kind": last_uplink.get("type"),  # 'wire' or 'wireless'
            }

        devices.append({
            "id": mac,
            "name": mac_to_name.get(mac),
            "type": d.get("type"),
            "model": d.get("model"),
            "mac": mac,
            "ip": d.get("ip"),
            "version": d.get("version"),
            "adopted": d.get("adopted"),
            "disabled": d.get("disabled", False),
            "uplink": uplink,
            "port_count": len(d["port_table"]) if isinstance(d.get("port_table"), list) else None,
            "ports": extract_device_ports(d, profiles_by_id),
            "site_id": d.get("site_id"),
            "site_name": site_name_by_id.get(d.get("site_id")),
        })
    return devices, mac_to_name


def extract_networks(networkconf_docs):
    networks = []
    for n in networkconf_docs:
        networks.append({
            "id": n.get("_id"),
            "name": n.get("name"),
            "purpose": n.get("purpose"),
            "vlan": n.get("vlan"),
            "vlan_enabled": n.get("vlan_enabled", False),
            "subnet": n.get("ip_subnet"),
        })
    return networks


def extract_port_profiles(portconf_docs, networks):
    """Reshape the `portconf` collection — UniFi's switch-port profiles — into
    a flat list with VLANs resolved from networkconf ids to real VLAN numbers.

    A profile is what the controller actually assigns to a switch port, so this
    is the table that turns a port override's `portconf_id` into "VLAN 40 /
    Voice". `forward` is UniFi's own word for the port's mode: 'native' (a
    plain access port on the native network), 'customize' (native + an explicit
    tagged set — a trunk), 'all' (every VLAN tagged), or 'disabled'.

    Neutral by design: a port being on VLAN 1, or trunking everything, is a
    fact here — whether that's wrong for this site is /mapnetwork's call."""
    net_by_id = {n["id"]: n for n in networks}

    def _vlan(net_id):
        n = net_by_id.get(net_id)
        if not n:
            return None, None
        # vlan is absent/None on the default (untagged) network — report it as
        # VLAN 1 only when the network itself says so, never by assumption.
        return n.get("vlan"), n.get("name")

    profiles = []
    for p in portconf_docs:
        native_vlan, native_name = _vlan(p.get("native_networkconf_id"))
        tagged = []
        for nid in (p.get("tagged_networkconf_ids") or []):
            v, name = _vlan(nid)
            if v is not None or name:
                tagged.append({"vlan": v, "network": name})
        voice_vlan, voice_name = _vlan(p.get("voice_networkconf_id"))
        profiles.append({
            "id": p.get("_id"),
            "name": p.get("name"),
            "forward": p.get("forward"),
            "native_vlan": native_vlan,
            "native_network": native_name,
            "tagged_vlans": tagged,
            "tagged_mode": p.get("tagged_vlan_mgmt"),
            "voice_vlan": voice_vlan,
            "voice_network": voice_name,
            "poe_mode": p.get("poe_mode"),
            "isolation": bool(p.get("isolation")),
            "stp_port_mode": p.get("stp_port_mode"),
            "port_security_enabled": bool(p.get("port_security_enabled")),
        })
    return profiles


def extract_device_ports(device_doc, profiles_by_id):
    """Per-port VLAN/PoE config for one device.

    Two sources, merged, nearest-wins:
      - `port_overrides` — the admin's explicit per-port config (this is where
        a port gets pinned to a VLAN profile, renamed, or PoE-forced). Only
        ports the admin actually touched appear here.
      - `port_table` — the live per-port state; on current controller versions
        each entry also carries the effective `portconf_id`, which covers ports
        running a profile inherited rather than overridden.

    A port appears in the output only when we know something real about it (a
    resolved profile, an explicit VLAN, a name, or a PoE mode) — this is not a
    row per physical port, so a 48-port switch with two configured ports emits
    two entries, not 48. `source` records which collection the entry came
    from so a consumer can tell an explicit override from an inherited profile.
    """
    ports = {}

    def _add(idx, doc, source):
        if idx is None:
            return
        prof = profiles_by_id.get(doc.get("portconf_id")) or {}
        entry = {
            "idx": idx,
            "name": doc.get("name") or prof.get("name") or None,
            "profile": prof.get("name"),
            "profile_id": doc.get("portconf_id"),
            "forward": prof.get("forward"),
            "native_vlan": prof.get("native_vlan"),
            "native_network": prof.get("native_network"),
            "tagged_vlans": prof.get("tagged_vlans") or [],
            "voice_vlan": prof.get("voice_vlan"),
            "poe_mode": doc.get("poe_mode") or prof.get("poe_mode"),
            "op_mode": doc.get("op_mode"),
            "isolation": prof.get("isolation"),
            "source": source,
        }
        known = any(entry[k] is not None for k in
                    ("profile", "native_vlan", "poe_mode", "op_mode")) or \
            bool(entry["name"]) or bool(entry["tagged_vlans"])
        if not known:
            return
        # port_overrides wins over port_table for the same port index.
        if idx in ports and ports[idx]["source"] == "port_overrides":
            return
        ports[idx] = entry

    for o in (device_doc.get("port_overrides") or []):
        if isinstance(o, dict):
            _add(o.get("port_idx"), o, "port_overrides")
    for p in (device_doc.get("port_table") or []):
        if isinstance(p, dict):
            _add(p.get("port_idx"), p, "port_table")

    return [ports[k] for k in sorted(ports)]


def extract_wlans(wlanconf_docs, networks):
    net_name_by_id = {n["id"]: n["name"] for n in networks}
    wlans = []
    for w in wlanconf_docs:
        wlans.append({
            "name": w.get("name"),
            "enabled": w.get("enabled", False),
            "security": w.get("security"),
            "hidden": bool(w.get("hide_ssid")),
            "vlan": w.get("vlan"),
            "networkconf_name": net_name_by_id.get(w.get("networkconf_id")),
        })
    return wlans


# Networking/security-appliance vendor keywords to match against a client's
# own `oui` string (UniFi's client-side vendor guess — a different source
# than _OUI_VENDORS above, deliberately not cross-checked against it). Purely
# a keyword flag: mechanical, not a verdict. A "client" whose oui matches one
# of these is often actually infrastructure (a gateway/firewall/AP that UniFi
# never adopted, showing up as a plain client because nothing tells the
# controller otherwise) rather than a real end-user device — but that's a
# judgment call for whoever builds the map, made against the rest of the
# site's evidence (which port, which subnet, whether the same vendor shows
# up on two different upstream devices — a classic multi-leg-appliance tell).
_INFRA_VENDOR_KEYWORDS = [
    "sonicwall", "fortinet", "cisco", "juniper", "mikrotik", "aruba",
    "palo alto", "meraki", "netgear", "tp-link", "sophos", "watchguard",
    "check point", "barracuda", "netgate", "pfsense", "ubiquiti", "zyxel",
    "huawei", "edgecore", "f5 networks",
]


def _is_infra_vendor_oui(oui):
    if not oui:
        return False
    o = oui.lower()
    return any(k in o for k in _INFRA_VENDOR_KEYWORDS)


def extract_clients(user_docs):
    """Reshape the `user` (client) collection into a neutral client list,
    each tagged with which adopted UniFi device it was last seen behind.

    Wired clients carry last_uplink_mac/last_uplink_name/last_uplink_remote_port
    (switch + port). Wireless clients carry last_uplink_mac/last_uplink_name
    (AP) plus last_radio, no port — WiFi doesn't have one. A handful of wired
    records (typically long-disconnected/stale) have none of these fields at
    all; those come through with uplink: null rather than being dropped, so
    the gap is visible instead of silently disappearing.

    `oui` here is UniFi's own vendor string on the client doc — a different
    source than extract_devices'/build_discovered_devices' local OUI lookup
    table. Keep them separate; don't merge or cross-check one against the
    other, they're independent guesses from different code.
    """
    clients = []
    for u in user_docs:
        is_wired = bool(u.get("is_wired"))
        uplink = None
        if u.get("last_uplink_mac"):
            uplink = {
                "mac": u.get("last_uplink_mac"),
                "name": u.get("last_uplink_name"),
                "port": u.get("last_uplink_remote_port") if is_wired else None,
                "radio": u.get("last_radio") if not is_wired else None,
            }
        clients.append({
            "mac": u.get("mac"),
            "name": u.get("name") or u.get("hostname"),
            "hostname": u.get("hostname"),
            "is_wired": is_wired,
            "is_guest": bool(u.get("is_guest")),
            "oui": u.get("oui") or None,
            "possible_infra_vendor": _is_infra_vendor_oui(u.get("oui")),
            "ip": u.get("fixed_ip") or u.get("last_ip"),
            "uplink": uplink,
            "last_seen": u.get("last_seen"),
            "noted": bool(u.get("noted")),
            "blocked": bool(u.get("blocked")),
            "site_id": u.get("site_id"),
        })
    return clients


def compute_port_signals(devices, networks):
    """Mechanical per-port facts: which ports sit on a non-default network, and
    which ports trunk VLANs. Both are structure, not judgment — an access port
    on the server VLAN and a trunk to a closet switch are entirely normal."""
    default_vlans = {n.get("vlan") for n in networks
                     if (n.get("purpose") == "corporate" and not n.get("vlan_enabled"))}
    default_vlans.discard(None)

    off_default, trunks = [], []
    for d in devices:
        for p in d.get("ports") or []:
            if p.get("native_vlan") is not None and p["native_vlan"] not in default_vlans:
                off_default.append({
                    "device_name": d["name"], "port_idx": p["idx"],
                    "port_name": p["name"], "profile": p["profile"],
                    "native_vlan": p["native_vlan"],
                    "native_network": p["native_network"],
                    "forward": p["forward"],
                })
            if p.get("tagged_vlans") or p.get("forward") in ("all", "customize"):
                trunks.append({
                    "device_name": d["name"], "port_idx": p["idx"],
                    "profile": p["profile"], "forward": p["forward"],
                    "tagged_vlans": p.get("tagged_vlans") or [],
                })
    return off_default, trunks


def compute_signals(devices):
    """Surface raw structural/version facts. No severity, no opinions —
    just what's true, so the map-building step can decide what matters for
    this particular site."""
    unresolved_uplinks = []
    reciprocal_uplinks = []
    firmware_versions = defaultdict(list)
    disabled_devices = []
    no_uplink_devices = []

    by_mac = {d["mac"]: d for d in devices}
    seen_pairs = set()

    for d in devices:
        if d["version"]:
            firmware_versions[d["version"]].append(d["name"])
        if d["disabled"]:
            disabled_devices.append(d["name"])
        if d["uplink"] is None:
            no_uplink_devices.append(d["name"])
        elif d["uplink"]["name"] is None:
            unresolved_uplinks.append({
                "device_name": d["name"],
                "device_mac": d["mac"],
                "uplink_mac": d["uplink"]["mac"],
            })
        else:
            # Check for reciprocity: does the uplink target claim *this*
            # device as its own uplink? That's a graph-structural
            # contradiction (last_uplink data from the controller disagreeing
            # with itself) worth surfacing as fact, not something to silently
            # resolve one way or the other.
            other = by_mac.get(d["uplink"]["mac"])
            if other and other.get("uplink") and other["uplink"]["mac"] == d["mac"]:
                pair_key = tuple(sorted([d["mac"], other["mac"]]))
                if pair_key not in seen_pairs:
                    seen_pairs.add(pair_key)
                    reciprocal_uplinks.append({
                        "a_name": d["name"], "a_mac": d["mac"],
                        "b_name": other["name"], "b_mac": other["mac"],
                    })

    return {
        "unresolved_uplinks": unresolved_uplinks,
        "reciprocal_uplinks": reciprocal_uplinks,
        "firmware_versions": dict(firmware_versions),
        "disabled_devices": disabled_devices,
        "no_uplink_devices": no_uplink_devices,
    }


def main():
    if len(sys.argv) < 2:
        print("Usage: python3 extract.py <input.json> [output.json]", file=sys.stderr)
        sys.exit(1)

    input_path = sys.argv[1]
    output_path = sys.argv[2] if len(sys.argv) > 2 else input_path.rsplit(".", 1)[0] + ".extracted.json"

    data = load_export_documents(input_path)

    collections = split_collections(data)

    site_docs = collections.get("site", [])
    sites = [{"id": s.get("_id"), "name": s.get("desc") or s.get("name")} for s in site_docs]
    site_name_by_id = {s["id"]: s["name"] for s in sites}
    # `site` (singular) is kept for back-compat with single-site exports —
    # the overwhelmingly common case. Multi-site exports get the full list
    # in `sites`, and each device below is tagged with site_id/site_name so
    # nothing from a second (or third) site silently disappears.
    site_summary = sites[0] if sites else {"id": None, "name": None}

    # networks and port profiles must resolve before devices — per-port VLANs
    # come from portconf, whose VLANs come from networkconf.
    networks = extract_networks(collections.get("networkconf", []))
    port_profiles = extract_port_profiles(collections.get("portconf", []), networks)
    devices, mac_to_name = extract_devices(
        collections.get("device", []), site_name_by_id, port_profiles)
    wlans = extract_wlans(collections.get("wlanconf", []), networks)
    signals = compute_signals(devices)
    off_default, trunk_ports = compute_port_signals(devices, networks)
    signals["ports_off_default_vlan"] = off_default
    signals["trunk_ports"] = trunk_ports
    discovered_devices = build_discovered_devices(signals["unresolved_uplinks"])
    clients = extract_clients(collections.get("user", []))
    signals["clients_no_uplink"] = [c["name"] or c["mac"] for c in clients if c["uplink"] is None]
    signals["possible_infra_clients"] = [
        {"name": c["name"], "mac": c["mac"], "oui": c["oui"],
         "uplink_name": c["uplink"]["name"] if c["uplink"] else None,
         "uplink_port": c["uplink"]["port"] if c["uplink"] else None}
        for c in clients if c["possible_infra_vendor"]
    ]

    counts = {k: len(v) for k, v in collections.items()}

    output = {
        "site": site_summary,
        "sites": sites,
        "devices": devices,
        "networks": networks,
        "port_profiles": port_profiles,
        "wlans": wlans,
        "signals": signals,
        "discovered_devices": discovered_devices,
        "clients": clients,
        "counts": counts,
    }

    with open(output_path, "w") as f:
        json.dump(output, f, indent=2)

    print(f"Wrote {output_path}")
    if len(sites) > 1:
        print(f"  NOTE: export contains {len(sites)} sites — devices tagged "
              f"with site_id/site_name, don't assume they're all one network",
              file=sys.stderr)
    print(f"  {len(devices)} devices, {len(networks)} networks, {len(wlans)} wlans")
    configured_ports = sum(len(d.get("ports") or []) for d in devices)
    print(f"  {len(port_profiles)} port profiles, {configured_ports} configured "
          f"ports ({len(signals['ports_off_default_vlan'])} off the default "
          f"network, {len(signals['trunk_ports'])} trunking)", file=sys.stderr)
    print(f"  {len(signals['reciprocal_uplinks'])} reciprocal-uplink pairs, "
          f"{len(signals['unresolved_uplinks'])} unresolved uplinks, "
          f"{len(signals['no_uplink_devices'])} devices with no uplink, "
          f"{len(signals['disabled_devices'])} disabled")
    if discovered_devices:
        with_vendor = sum(1 for d in discovered_devices if d["vendor"])
        print(f"  {len(discovered_devices)} discovered (non-adopted) devices "
              f"from unresolved uplinks ({with_vendor} with a vendor guess)",
              file=sys.stderr)
    if clients:
        with_uplink = len(clients) - len(signals["clients_no_uplink"])
        wired_n = sum(1 for c in clients if c["is_wired"])
        print(f"  {len(clients)} clients ({wired_n} wired, "
              f"{len(clients) - wired_n} wireless), {with_uplink} with a "
              f"resolved uplink device", file=sys.stderr)
        if signals["possible_infra_clients"]:
            print(f"  {len(signals['possible_infra_clients'])} client(s) with "
                  f"a networking/security-vendor OUI — possible unadopted "
                  f"infrastructure, not real end-user clients; see "
                  f"signals.possible_infra_clients", file=sys.stderr)


if __name__ == "__main__":
    main()
