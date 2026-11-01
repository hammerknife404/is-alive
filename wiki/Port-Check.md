Concurrent TCP connect scan of one host.

```
Host [dc01.corp.local] [Start]  Ports [53,88,135,389,445] [Presets ⏷]
Speed [Normal ⏷]  [ ] Show closed/filtered  [ ] Prefer IPv6
10.0.0.10   5 open · 14 / 14 scanned
53   dns       open   0.4 ms
88   kerberos  open   0.5 ms
```

## Port syntax

Comma, space or semicolon separated; ranges with `-`. Duplicates are removed and the list is sorted.

```
22,80,443          8000-8010          1-1024          22 80 443;3389
```

## Presets

| Preset | Ports |
|---|---|
| Common | 21-23, 25, 53, 80, 110, 135, 139, 143, 389, 443, 445, 465, 587, 636, 993, 995, 1433, 1723, 3306, 3389, 5432, 5900, 5985, 8080, 8443 |
| Windows / AD | 53, 88, 135, 139, 389, 445, 464, 636, 3268, 3269, 3389, 5985, 5986, 9389 |
| Web | 80, 443, 8000, 8008, 8080, 8443, 8888 |
| Firewall / VPN | 22, 443, 1723, 4433 (SonicWall), 8443, 10443 (Fortinet) |
| Mail | 25, 110, 143, 465, 587, 993, 995 |
| Well-known 1-1024 | 1-1024 |

Hover a preset to see its list. Results show the well-known service name beside each port.

## States

| State | Means |
|---|---|
| **open** | The connect succeeded: something is listening |
| **closed** | Refused or reset: the host is up, nothing listens there |
| **filtered** | No answer before the timeout: a firewall dropped it, or the host is down |
| **error** | Anything else (e.g. no route); the text says what |

Only **open** is shown unless **Show closed/filtered** is ticked. On Windows a port that times out is tried once more before it's called filtered, so a single lost SYN isn't misread.

## Limits

- **TCP only.** UDP services (IKE 500/4500, OpenVPN 1194, SNMP 161, DNS over UDP) can't be tested by a connect scan and would always read filtered, so they're left out of the presets.
- It's a port scan. EDR/IDS may flag it; see [Scan Speed](Scan-Speed) for throttling.

## Export

CSV columns: `host, ip, port, state, service, rtt_ms, error` (closed/filtered rows included).
