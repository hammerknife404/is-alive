Concurrent TCP connect scan — lists, ranges, presets (Common, Windows/AD, Web, Firewall/VPN, Mail, 1-1024) with service names. On Windows a port that times out gets one more try, so a lost packet isn't read as filtered. TCP only: UDP services such as IKE (500/4500) can't be tested.

Like every tab: the run bar (the main field first, **▶ Start** right after it), the other settings folded under **Options** with their values shown as chips, a status row under it with what the run found on the left and **Copy** / **Save…** on the right, and a short explanation of what to do before the first run. Enter in the main field starts a run; Esc stops it.

## Scan speed

Port Check and Subnet Sweep have a **Speed** preset. Threads only cap how many probes are in flight; the **rate** (probes started per second, evenly spaced) is what stops a scan from bursting against hosts that answer fast.

| Preset | Rate | In flight | 1-1024 ports | /24 sweep |
|---|---|---|---|---|
| Gentle | 10/s | 4 | ~1m 43s | ~26 s |
| Normal (default) | 50/s | 16 | ~21 s | ~5 s |
| Fast | 500/s | 64 | ~2 s | <1 s |
| Unthrottled | none | 256 | tens of ms | tens of ms |

Any TCP connect or ICMP sweep is a scan by definition; throttling reduces noise and load but won't make it invisible to IDS/EDR. For sanctioned use, exclude the tool or the source host in the security stack.

**CSV columns** (Results → Export → CSV): host, ip, port, state, service, rtt_ms, error (closed/filtered included). See [[Results]].
