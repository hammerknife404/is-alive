Port Check and Subnet Sweep have a **Speed** preset. Threads only cap how many probes are in flight; the **rate** (probes started per second, evenly spaced) is what stops a scan from bursting against hosts that answer fast.

| Preset | Rate | In flight | 1-1024 ports | /24 sweep |
|---|---|---|---|---|
| Gentle | 10/s | 4 | ~1m 43s | ~26 s |
| Normal (default) | 50/s | 16 | ~21 s | ~5 s |
| Fast | 500/s | 64 | ~2 s | <1 s |
| Unthrottled | none | 256 | tens of ms | tens of ms |

Any TCP connect or ICMP sweep is a scan by definition; throttling reduces noise and load but won't make it invisible to IDS/EDR. For sanctioned use, exclude the tool or the source host in the security stack.

ICMP goes through the Windows IP Helper API (`IcmpSendEcho2` / `Icmp6SendEcho2`), so it works as a normal user — no raw sockets.
