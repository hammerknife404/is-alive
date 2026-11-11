Every finished run from every tab is listed here, newest first, under the day it ran, with what's still running at the top. Each row has the tool, its target, the time and a few words on what it found (*2 open of 14 checked*, *37 alive*, *Down 412 · up 38 Mbps*, *1 failed · 2 warnings*). Click one to open it beside the list: when it ran and how long it took, **Run again** (same tool and target, with its tab's settings as they are now; not for Lookup), **On map** when the target is an address, **Copy**, **Save…**, **Export**, **Remove** (click twice: takes the run out, and out of the site's saved runs), and the output as printed or its data as a **Table**.

**Compare ▾** (beside Export) lists the other runs of the same tool, those of the same target first; pick one and the **Compare** view shows what's **New**, **Gone** and **Changed** between them, the earlier run as before: a sweep's hosts by IP (MAC, vendor, name, open ports), Port Check's ports, Traceroute's hops, DNS answers, Domain Health's checks, and Speed Test's and Ping's figures side by side with the difference. Reply times aren't counted as changes. **Copy** copies the comparison.

The **Filter** box at the top of the list matches tool names and output text. **All runs ▾** copies, saves or exports every run listed (CSV, JSON, Markdown or HTML: the filter decides which), turns **Keep with site** on or off (a site keeps its Results in its folder from one visit to the next; Scratch keeps them for a day, as **Keep for a day**, so a crash or a flat battery doesn't lose the visit but they don't follow you to the next client) and has **Clear all**, which asks for a second click.

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
| Domain join | section, field, value (the first rows: `join`, `kind` and `domain`) |

Exporting the whole log gives one CSV: a `tool` column plus every tool's columns (shared ones like `ip` and `rtt_ms` line up), so filter on `tool` in Excel. Runs that failed before producing results carry their text in an `output` column.
