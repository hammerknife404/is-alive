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
- Default theme is **RGB** (`defaultTheme: 'rgb'`); a theme the viewer already saved in their browser wins.
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
     `label`, add a node `flags[]` entry on both ends, and one `med` ACTIONS
     entry per site covering every pair there — "controller uplink data
     disagrees with itself; direction shown is a layout assumption; verify
     cabling on-site." Raise it to `high` only when resolving the pair
     leaves part of the site disconnected from its root. The Contoso Health
     example (edges `e35`, `e44`) shows the pattern.
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
  120 nodes.
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

If `validate_map.py` is missing from the deployment, fall back to Node.js:
slice from the start marker's opening `/*` (not from after the marker —
that hides a splice that commented the data out) up to the END marker, load
it with
`new Function(block).call(ctx)` (the block's `this.X =` statements then land
on `ctx` — don't rewrite them), and walk `ctx.NODES`/`EDGES`/`ACTIONS`/
`ZONES`/`SITES` for the errors above. Flag the missing script as a
deployment gap in the reply rather than treating the fallback as
equivalent long-term.

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
