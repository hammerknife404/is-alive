# Beacon cross-check (Step 2.5)

Beacon syncs IT Glue **Configurations** into a queryable graph. For a map that
gives three things no other input can: identity for infrastructure that was
only inferred (a server seen as a client-table MAC), documentation drift
(Glue says one WAN IP, the firewall says another), and coverage gaps (live
gear nobody documented). Cost is 3–4 tool calls per map.

Origin case: Byrnes Mechanical, 2026-10-06. Without this check the map split
one server into two "possibly different" HPE nodes, missed an expired
warranty, and missed a wrong Shreveport WAN IP in Glue. With it, all three
were resolved in four calls.

## What Beacon does NOT cover

- **Only Glue Configurations.** Glue documents, passwords and most flex
  assets are not synced (check `records_synced` in the status call). A
  wrong IP stored in a password entry or doc is invisible here. If the user
  reports a documented value that failed (an IP, a hostname), treat what
  they report as one more claimed source.
- **No warranty or lifecycle enrichment unless ScalePad is `configured`.**
  Warranty dates come only from what a tech typed into Glue.
- **No live state.** Every Beacon fact is documentation.

## Procedure

1. **Status** — `get_server_status`, silently. Continue only if
   `itglue_config` is `configured`. If it is `stale`, continue but caveat any
   drift finding ("Glue last synced <date>").
2. **Resolve the client** — `search_graph` with `node_types:["company"]`.
   Take the exact `company_identifier` from the returned node. Never type it
   from the client's name: identifiers carry typos (Byrnes is stored as
   `Byrnes Mechnical`) and the match is exact.
3. **Learn this client's vocabulary** — `aggregate_nodes`,
   `group_by:"config_kind"`, `source:"authoritative"`, with **no filters**.
   `config_kind` values are lowercase and coarse: firewalls are `router` and
   APs are `switch`. The finer label is `config_type` (`Firewall`,
   `Access Point`, `Managed Server`). A filtered first query that guesses
   the vocabulary returns zero and looks like "nothing documented". It isn't.
4. **Pull infrastructure** — `query_nodes`, `source:"authoritative"`,
   `config_kind` `in` every kind from step 3 except workstation, printer and
   peripheral.

If a call errors (auth expires roughly daily), skip the check and say so in
the reply. Never block a map on Beacon.

## Matching Glue records to map nodes

Match in this order, and stop at the first hit:

1. **Serial**, normalized: strip dashes and spaces, uppercase.
   `2CB8-EDDA-6E30` = `2CB8EDDA6E30`. This is the most reliable key. An iLO
   hostname often embeds the host's serial (`ILOMXQ0000001` → serial
   `MXQ0421451`), which ties an out-of-band interface to its server.
2. **MAC**, normalized to lowercase colon form. **SonicWall:** Glue usually
   records the WAN-side MAC, which is X0's MAC + 1 in the last octet
   (`…:6e:31` vs the `…:6e:30` the switches learn). Accept ±1 on the last
   octet for firewalls only.
3. **IP, LAN-side only, as a weak hint.** Never match on WAN IP: that is the
   field most likely to have drifted, so using it as a key hides the drift.

**Never match on name.** Glue names (`NetTech SECaaS SonicWALL - Main`)
and controller names are written independently.

## What to do with the results

**Precedence.** Beacon labels Glue `authoritative`. That label ranks Glue
above ticket-derived snippets **inside Beacon**. In this skill's
cross-source conflict ladder, Glue is a **claimed** source: it ranks below
the controller and firewall config for every network fact (IP, VLAN, port,
firmware, WAN). It ranks highest only for facts nothing else carries:
model, serial, OS, warranty, purchase date, install date, location.

- **Identity upgrade.** If a matched record names an inferred node (a
  `possible_infra_clients` promotion, a client-table server), use the Glue
  model/OS as the node's `kind`/`details` and keep the live MAC/port
  evidence. Merge nodes that turn out to be one device (a server and its
  iLO). Add a `details` line citing Glue via Beacon.
- **Documented, observed-only-as-client.** A Glue server, firewall or
  switch whose MAC or IP appears in the client table becomes a node, even
  with clients off the map. It counts against the budget like any other
  infrastructure.
  - Example: a Hyper-V VM whose `00:15:5d` MAC sits on the same switch
    port as a documented host. Edge it from that host, dashed, labelled
    `Hyper-V guest`, and say in `details` that the hosting is inferred.
- **Documented, not observed anywhere.** No node. List it in one `low`
  ACTIONS entry ("documented in Glue, not seen live — retired or not
  adopted?").
- **Drift** (a network fact differs between Glue and live config): draw the
  live value, add the `Sources disagree: …` details line, and file it
  under the conflict ladder's severity rules. A WAN/management IP mismatch
  is `med`, because it blocks remote access. A LAN IP mismatch on an AP or
  switch is `low`.
  - Collapse all drift into as few ACTIONS as reads cleanly. Two wrong
    firewall WAN records is one finding, not two.
- **Coverage gap** (live infrastructure with no Glue record): one `low`
  ACTIONS entry naming the undocumented devices. Gateways and switches
  first.
- **Glue-only facts** that change risk, such as an expired warranty on a
  server or core device: a `flags[]` entry on the node plus an ACTIONS
  entry. A warranty date by itself is `med` on a server or firewall and
  `low` on an AP.
- **Agreement** is worth one `details` line on firewalls ("Glue WAN
  matches"). It tells the reader the value was checked, not just copied.

Add `IT Glue via Beacon` to the sources listed in `META.subtitle` when the
check ran.
