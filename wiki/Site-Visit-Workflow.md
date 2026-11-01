A typical on-site run, from plugging in to handing the data to Claude for a proper map.

```
 plug in ─▶ Sweep ─▶ Config Export ─▶ Map (check) ─▶ Notes ─▶ Site Bundle ─▶ /unifidump + /mapnetwork
             │          (UniFi / SonicWall / Meraki)          │
             └─ Check mgmt ports                               └─ redacted automatically
```

## 1. Baseline the line

- [Speed Test](Speed-Test): run it first, while the line is quiet. A bufferbloat grade of B or worse explains most "calls break up" tickets.
- [Ping](Ping) the gateway and `1.1.1.1` with Count 0 on a spare tab while you work; loss shows up over time.

## 2. Find what's there

- [Subnet Sweep](Subnet-Sweep) the LAN (`This network` in [Subnet Calc](Subnet-Calc) → **Sweep**). Tick **Check mgmt ports** if a port scan is sanctioned: open 22/443/8443/4433 separate infrastructure from clients.
- Save the **Inventory…** if you're not making a Site Bundle.

## 3. Load the configs

Drag them onto the window ([Config Export](Config-Export)):

| Gear | File |
|---|---|
| UniFi | `.unf` site export or autobackup, or `.unifi` console backup |
| SonicWall | `.exp` settings export, or a Tech Support Report `.wri` (adds LLDP neighbours) |
| Meraki MX | JSON from [`meraki-export.ps1`](https://github.com/hammerknife404/is-alive/blob/main/scripts/meraki-export.ps1) |
| Earlier visit | the `/mapnetwork` `.dc.html` map |

Secrets are stripped on this machine; the summary lists every field removed.

## 4. Sanity-check the Map

The [Map](Map) tab draws what the app knows. Use it to spot gaps (an *Unidentified upstream device*, hosts that didn't answer) and fix them while you're still on site. Click a card → **Ping / Ports / Trace** for quick checks.

## 5. Take notes

[Notes](Notes): **Time** and **Host info** stamp where and when; anything from a loaded backup's secrets is redacted as you type. **Ctrl+R** redacts a selection by hand.

## 6. Bundle and hand off

**Site Bundle…** (top right) → name the site, add known problems, choose UniFi client handling (*anonymized* by default) → save. The kickoff prompt is copied to the clipboard: attach the zip in Claude, paste, done. See [Site Bundle](Site-Bundle).

## 7. Mail problems?

[Domain Health](Domain-Health) on the client's domain covers SPF/DMARC/DKIM, MX, SMTP STARTTLS, blacklists, MTA-STS, DNSSEC and domain expiry in one pass.
