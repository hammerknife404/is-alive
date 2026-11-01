```
Host [10.0.0.1      ] [Start]  | (ICMP) TCP
Count [0] (0 = continuous)  Interval [1000 ms]  Timeout [..]  Size [32 B]  [ ] Prefer IPv6
10.0.0.1   Sent 42  Received 42   Loss 0%   min 0.3 / avg 0.4 / max 1.9 ms
▁▁▂▁▁▁▃▁▁▁▁▂  (live latency graph)
```

## Modes

| Mode | Sends | Use when |
|---|---|---|
| **ICMP** | Echo request (IPv4 or IPv6) | Normal ping |
| **TCP** | A TCP connect to **Port**, timed | The host or a firewall drops ICMP (Windows firewall default, many cloud hosts). An answer, open or refused, proves the host is up |

## Options

| Field | Meaning |
|---|---|
| **Count** | Pings to send; **0** runs until **Stop** |
| **Interval** | Time between pings (100 ms to 60 s) |
| **Timeout** | How long to wait for each reply (100 ms to 30 s) |
| **Size** | ICMP payload bytes (0 to 65,500). Large sizes test MTU and fragmentation |
| **Prefer IPv6** | When a name has both A and AAAA, use AAAA |

The header shows the resolved address, sent / received, loss (coloured by severity) and min / avg / max. Failed replies are red in the log.

## How it pings

- **Windows**: the IP Helper API (`IcmpSendEcho2` / `Icmp6SendEcho2`). Normal user, no raw sockets.
- **Linux**: unprivileged ICMP datagram sockets; falls back to the system `ping` (from `/usr/sbin`, `/sbin`, `/usr/bin` or `/bin` only, never PATH) if `ping_group_range` excludes you.
- **Other**: the system `ping` (development only).

## Export

CSV columns: `target, mode, port, seq, from, status, rtt_ms, ttl, bytes`. See [Interface and Export](Interface-and-Export).

## Tips

- Long-running continuous ping while someone reproduces a dropout: set Count 0, leave it on another tab. Results keep collecting while you work elsewhere.
- Jitter and VoIP quality under load are the [Speed Test](Speed-Test)'s job, not Ping's.
