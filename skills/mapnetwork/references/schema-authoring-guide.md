# Network Topology Map — Authoring Guide

A reusable, data-driven engine for mapping client / site networks. The visuals,
theming, touch gestures, pan/zoom, search, filtering, flow animation, and mobile
layout are all **fixed and generic** — to map a new site you only supply **data**.

---

## The fastest way to make a new map

**Just give Claude the site info.** Paste a device list, an nmap/scan dump, a
spreadsheet, a firewall config, or plain notes ("router is a UniFi UDM at .1, two
switches, a NAS at .10, ~12 wifi devices…") and ask for a map. Claude fills in the
SITE DATA block for you and produces a finished, self-contained file.

The rest of this guide documents the schema so you (or Claude) can do it by hand.

---

## Three rules that outrank the schema

1. **120 NODES, hard ceiling. 40–90 is the target.** Infrastructure first
   (gateways, switches, APs, servers, discovered/unadopted gear). A map is a
   decision tool, not an inventory export.
2. **Client devices are off by default.** Workstations, phones, printers, IoT,
   guest devices, the whole DHCP/controller client table — only when the user
   asks, and only for the scope they asked about.
3. **ZONES are roles, SITES are places.** Never spend the zone colour channel
   on geography, and never generate a per-site hue ramp — use the palette below.

---

## How it's structured

- **`Site Map Template.dc.html`** — the engine holding a small **sample site**
  that uses *automatic layout* (no coordinates). Duplicate this to start a new map.
- **`example-Contoso_Health.dc.html`** — the same engine holding a fully worked
  multi-site client example (23 sites, 81 infrastructure nodes, collapsible
  sites, 9 findings, clients deliberately not mapped).
- **`example-Acme_Distribution_LLC.dc.html`** — a single-campus worked example
  authored before SITES existed (sites are optional; that file still renders).
- Each map is one self-contained file. To make a new client map: **duplicate the
  template, replace the SITE DATA block, rename the file** — underscores, no
  spaces (a space in the filename breaks the engine's self-refetch in some
  browsers and the map renders as raw source).

Inside each file, the data lives in the logic class between these markers:

```
/* ===================== SITE DATA ===================== */
   this.META   = { … }
   this.SITES  = { … }     // optional
   this.ZONES  = { … }
   this.NODES  = [ … ]
   this.EDGES  = [ … ]
   this.ACTIONS= [ … ]
   this.BANDS  = [ … ]
/* =================== END SITE DATA =================== */
```

Replace everything between the markers, keeping both marker lines. Each marker is a single closed comment, so the data goes on the line after the start marker. (Maps built before the markers were normalized have a multi-line start banner — there, insert after the banner's closing `*/`.) Don't touch anything outside them.

---

## Schema

### META
```js
this.META = {
  title:    'Acme HQ — Network',                        // header title
  subtitle: '10.0.0.0/24 · 3 VLANs · clients not mapped' // small caption (desktop only)
};
```
Say "clients not mapped" in the subtitle whenever they aren't, so nobody reads
the map as a complete device census.

### SITES (optional — geography)
One entry per physical location. Every node then carries `site:'<key>'`.

```js
this.SITES = {
  upstream:  { name:'Upstream / unmanaged' },
  alpha_nb: { name:'Alpha New Building' },
  november:     { name:'November' }
};
```

- Sites render as **collapsible containers**. A map with **more than 3 sites
  opens with every site collapsed** — each site is one summary card showing its
  device count and flag count, and its links fold into one trunk per neighbour.
  Expand from the sidebar row, the container title, or the collapsed card.
- Tapping a site (card or container title) selects the **whole group**: every
  member highlights, every link crossing the boundary animates, and the detail
  panel lists the site's composition plus each crossing link (`via <member> ·
  <port>`), upstream and downstream marked. That's the "what does this site
  connect to" question answered in one tap.
- Devices that aren't at a client location (an unadopted upstream firewall, a
  carrier handoff, an unresolved MAC) belong in an `upstream` site named
  something honest like *Upstream / unmanaged* — don't pretend they're on site.
- Omit `SITES` entirely for a single-location map. Everything works as before
  and the layout toggle drops the Sites mode.

### ZONES (role / segmentation — never geography)
The key (e.g. `gateway`) is what each node references in its `zone`. Each zone
needs a **dark** and **light** accent colour (used for cards, edges, legend, and
the detail panel in each theme).

```js
this.ZONES = {
  gateway:      { name:'Gateway / firewall', vlan:'edge',              dark:'#5b9dff', light:'#2563c9' },
  distribution: { name:'Switches',           vlan:'wired',             dark:'#a98bff', light:'#6d44d9' },
  access:       { name:'Access points',      vlan:'wireless',          dark:'#5fcf8a', light:'#1c9b54' },
  voip:         { name:'Voice',              vlan:'VLAN 20',           dark:'#e0b34d', light:'#a9790c' },
  iot:          { name:'IoT / devices',      vlan:'VLAN 30 · isolate', dark:'#f0944a', light:'#cf5d18' },
  external:     { name:'Unidentified / external', vlan:'MAC only',     dark:'#8a93a3', light:'#5d6878' }
};
```

**The palette (dark / light) — use these six, nothing else:** blue
`#5b9dff`/`#2563c9`, purple `#a98bff`/`#6d44d9`, green `#5fcf8a`/`#1c9b54`,
amber `#e0b34d`/`#a9790c`, orange `#f0944a`/`#cf5d18`, grey `#8a93a3`/`#5d6878`.
**Cap at ~8 zones.** Twenty auto-generated pastels make the legend unreadable
and the filter useless — if you find yourself generating colours in a loop,
you're using zones for the wrong dimension (you want SITES).

If a node references a zone key that isn't defined here, the engine logs a console
warning and files it under a grey **Unzoned** zone instead of failing — but fix the
key; it's always a typo.

### NODES
One per device. **`x`/`y`/`w`/`h` are optional** — omit them on *every* node and the
engine auto-lays the whole map out by connection depth (root at top). Provide them
on every node only if you want a hand-tuned layout.

```js
{
  id:'fileserver',                 // unique, used by edges + actions
  site:'alpha_nb',                // a SITES key (omit on maps with no SITES)
  zone:'server',                   // a ZONES key — role, not location
  kind:'FILE SERVER',              // small mono label at the top of the card.
                                   // ALL CAPS, <= 18 characters. Every card is the
                                   // same 212x112 box, so a longer kind truncates to
                                   // an ellipsis rather than reflowing the card.
  label:'File Server',             // card title
  sub:'192.168.1.10',              // IP / accent-coloured subtitle
  details:[                        // bullet list in the detail panel
    'Windows Server', 'SMB shares', 'Nightly backup to NAS'
  ],
  flags:[                          // action items specific to this node (⚠ badge + pulse)
    'No off-site backup yet'
  ],
  chips:['SMB','.10']              // tiny tags on the card
  vlans:[10]                        // OPTIONAL — real VLAN(s) this device carries (from
                                    // /unifidump's port/network data, or a known trunk
                                    // config). Auto-renders a "VLAN 10" (or "VLANs
                                    // 10/20/30") chip on the card, and makes the device
                                    // toggle in the sidebar's VLAN filter — independent
                                    // of its zone. Omit when segmentation isn't actually
                                    // known; never guess a VLAN from a name or IP.
  // x, y, w, h        // OPTIONAL — omit for auto-layout
}
```
`details`/`flags`/`chips`/`sub` may be omitted (they default to empty).
A node with a non-empty `flags` array gets a ⚠ badge and a subtle pulse. The
header's "action items" count is the site-wide ACTIONS total (below).

A node whose `site` is missing or unknown lands in an auto-created
**Unassigned** site with a console warning.

#### Discovered / unadopted nodes (from `/unifidump`'s `discovered_devices`)

Equipment the input evidence proves *exists* but doesn't fully identify —
most commonly an unresolved uplink MAC from `/unifidump` — still gets a real
NODE, not just a flag on whatever's downstream of it. There's no dedicated
schema field for "unadopted" (node card styling is driven purely by `zone`,
nothing else) — so use the **zone itself** to carry that visual distinction,
and reserve `flags`/red for real faults, not for "not managed by us":

```js
this.ZONES = {
  // ...your normal zones...
  external: { name:'Unidentified / external', vlan:'', dark:'#8a93a3', light:'#5d6878' }
};
```

```js
{
  id:'discovered_e4382f8bdc6b',        // stable id derived from the MAC
  site:'upstream',                      // not at a client location
  zone:'external',                      // <- the grey zone above, not a guess at its real zone
  kind:'FORTINET DEVICE',               // vendor if known, else 'UNKNOWN DEVICE'
  label:'Unidentified Upstream Device',
  sub:'e4:38:2f:00:00:01',
  details:[
    'Seen upstream of Echo Clinic (unresolved uplink)',
    'Vendor: Fortinet (OUI match — unverified, confirm on-site)',
    'Not adopted by this UniFi controller'
  ],
  chips:['external','MAC only']
  // no flags[] here unless there's an actual fault to report
}
```

- **`zone:'external'`** (grey, defined once per map) is what actually renders
  as visually distinct given the current engine — the card's left-border
  accent and legend entry come straight from the zone color, no other hook
  exists. This also means it shows up as its own filterable group in the
  zone legend, which is a bonus, not a downside — "show me everything we
  don't manage" becomes a single click.
- **Don't use `flags`/red for "unadopted."** Red already means "actionable
  risk" in this UI. Reserve a discovered node's `flags` for an actual fault
  you can see (e.g. the same MAC reported upstream of two unrelated sites —
  that's real, flag it normally).
- **Chips carry the uncertainty:** `['external']`, `['MAC only']` (no vendor
  match), or `['unresolved']`. Don't invent a model/role chip
  (`'firewall'`, `'FortiGate 60F'`) from vendor + position alone — only add
  it if some other piece of evidence in the input actually says so.
- **Edge it in normally** — `from` = the discovered node (it's upstream by
  definition, since this is how `/unifidump` found it), `to` = the adopted
  device that reported the uplink, `cls:'backbone'` or `'wired'` as usual.

### Client devices (CD) — opt-in, then cap at 10

"Client device" = end-user/endpoint hardware hanging off a switch or AP
(workstations, phones, printers, IoT, guest devices) — as opposed to
infrastructure (switches, routers, APs, servers), which always gets its own
node regardless of count.

**Default: don't map them at all.** They are the reason maps become unusable
(a controller export routinely carries 500–1,500 of them). Say so in
`META.subtitle` and move on.

**When the user asks for them**, for the scope they asked about:

- Give each one a real NODE like any other device — normal
  `site`/`zone`/`kind`/`label`/`sub`/`details`/`chips`.
- **Cap: 10 individual client nodes per immediate upstream device** (per
  switch, per AP — whichever they're one hop off of). If 10 or fewer share
  that upstream, show them all, no collapse.
- **11+ sharing the same upstream:** show the top 10 as individual nodes,
  collapse the rest into ONE overflow node. Priority for which 10 stay
  individual (deterministic — don't reshuffle on a rebuild):
  1. anything with a real flag/fault
  2. anything with a resolved hostname/known identity (vs. MAC-only/unknown)
  3. remaining, in input order
- Stay inside the 120-node budget regardless. If the requested scope won't
  fit, map what does and say what you left off.

**Overflow node:**
```js
{
  id:'sw2_overflow',                 // '<upstream_id>_overflow'
  site:'alpha_nb',
  zone:'trusted',                    // same zone the collapsed devices are in
  kind:'ADDITIONAL DEVICES',
  label:'+14 more',
  sub:'via Switch 2',
  details:[                          // this is the "expand" — no UI change needed,
    'laptop-jsmith · 192.168.1.42',  // tapping the node already opens the detail
    'printer-3fl · 192.168.1.51',    // panel per the engine's built-in behavior
    // ...one line per omitted device, whatever identity info is known
  ],
  chips:['collapsed','14 devices']
  // no flags[] unless one of the collapsed devices has an actual fault
}
```
Edge it in normally (`wifi`+`dashed` or `wired`, matching how the real
devices would connect).

**Don't fabricate identities.** If input only gives a headcount ("~30 wifi
clients," no individual list), that's not client identification — represent
it as a single node stating the uncertainty in `details`.

### EDGES
Links between nodes. Direction matters for the flow animation: list `from` as the
**upstream** side (closer to the internet/root) and `to` as the **downstream** side.

```js
{ id:'e2', from:'fw', to:'sw', cls:'backbone', label:'LAG' }
```
- `cls`: `'backbone'` (thick, always blue — internet→gateway spine) ·
  `'wired'` (zone-coloured solid) · `'wifi'` (zone-coloured, use with `dashed:true`)
- `dashed:true` — dashed line (wireless)
- `label` — optional small text on the line (port, link name)
- `anchor:'L'`/`'R'` — optional, fan two edges to the left/right of the same target
  (used when one device has two NICs into a switch)
- An edge whose `from`/`to` doesn't match any node `id` is **skipped with a console
  warning** rather than crashing the map — if a link is missing, check the console.

The engine computes each node's depth from the root by walking edges, which drives
both the auto-layout rows **and** the upstream/downstream flow direction. When a
site is collapsed, its internal edges disappear and its external edges re-point
at the site's summary card automatically — you author edges device-to-device and
never think about it.

### ACTIONS — the findings register
Site-wide review list. It renders **twice**: as the collapsible **findings band
across the top of the map** (severity counts + the list, worst first) and in the
sidebar. Tapping one highlights the related devices — and if they're inside a
collapsed site, it selects that site.

```js
{ id:'ai1', sev:'high', text:'File server has no off-site backup', nodes:['fileserver'] }
```
- `sev`: `'high'` (red) · `'med'` (amber) · `'low'` (grey)
- `nodes`: ids to focus when the item is tapped (can be empty `[]` for
  fleet-wide observations)
- Write each `text` as one actionable sentence: what's wrong, where, what to do.
  The band clamps to two lines, so lead with the finding, not the preamble.
- Be honest with severity, and order worst-first. Nine real findings beat
  thirty padded ones. Data-quality problems in the export itself
  (contradictory uplinks, stale MACs, unresolved clients) are legitimate
  findings — usually `med`/`low`.

### BANDS (manual layouts only)
Free-floating zone captions placed by absolute `x`/`y` on the canvas. Only useful
when you hand-position nodes. **Leave `this.BANDS = []` for auto-layout.**

```js
{ text:'IoT / Security · ⚠ NO ISOLATION', x:884, y:222, dark:'#f06a6a', light:'#cd3a2e' }
```

---

## Manual vs automatic layout

- **Automatic** (recommended for new sites): omit `x/y/w/h` on every node, set
  `this.BANDS = []`. The header shows a layout toggle:
  - **Sites** (only when `SITES` is declared; the default then) — one container
    per site, collapsed sites folded to a single card. The right default for
    anything multi-location.
  - **Cluster** — depth rows wrap into compact grid blocks; balanced and the
    least sprawling. Best default for a single site.
  - **Tree** — classic one-row-per-depth layered tree (can get very wide).
  - **Zones** — one container per zone; members grid inside it, and links that
    share an upstream device collapse into a single counted trunk line.

  Set the starting mode per map with the `defaultLayout` tweak (`site`/`cluster`/
  `tree`/`group`); a technician's runtime choice is remembered in localStorage,
  namespaced per map title.
- **Manual**: give every node `x/y/w/h` in a 0-based canvas
  coordinate space; the canvas auto-sizes to fit. Use BANDS to caption regions.
  Manual maps keep their fixed layout — the layout toggle is hidden.

The zone filter starts with **every zone visible**, derived automatically from
`ZONES`. You never declare which zones are on — just define them.

---

## Built-in features (no configuration needed)

Pan (drag), pinch / wheel zoom, fit-to-screen, light/dark toggle (persists),
search by name / IP / MAC (a collapsed site matches on its members' names too),
tap a zone to filter, **tap a VLAN to filter** (a separate sidebar list, shown
only once any node declares `vlans` — lets a trunk device's multiple VLANs and
a role's single zone be toggled independently), tap a node to trace its
connections (directional flow animation), tap a site or zone container to trace
the **whole group's** connections, expand/collapse sites individually or all at
once, the findings band, responsive phone/tablet/desktop layouts. The
**Tweaks** panel exposes default theme, default layout, curved vs straight
edges, show/hide the WiFi fan, and risk-pulse on/off.

Pan and zoom paint straight to the transform during a gesture and commit on
release, so a 100-card map stays responsive on a laptop trackpad.

---

## Sharing the file

The `.dc.html` is already a single self-contained file — engine and data are
inlined, fonts and React load from CDN at open-time. Hand the client that one
file; there's nothing else to attach. (It still needs internet access to pull
fonts/React from CDN on first open — it's not a fully offline artifact.)

## Default UX (engine behavior — not authored per map)
These come from the engine, so every new map inherits them from the template; never hard-code them in map data.
- **Theme:** RGB by default. A viewer's saved theme (per map, in localStorage) overrides it. Change only via the `defaultTheme` tweak.
- **Findings band:** collapsed on load; header click toggles. Keep ACTIONS severity-sorted and honest — the band's chips still show the counts while collapsed.
- **Sidebar sections:** Sites, Zones, VLANs and Action items each have a hide/show button; hidden state is session-only.
- **Panel collapse (desktop):** left sidebar → 44px rail (‹/›, map refits); right detail panel → header-only (▴/▾). Mobile keeps its drawer/sheet behavior.
