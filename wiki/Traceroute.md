Hop-by-hop path to a host using ICMP with increasing TTL.

```
Host [8.8.8.8] [Start]  Max hops [30]  Timeout [1000 ms]  [x] Resolve names  [ ] Prefer IPv6
Tracing 8.8.8.8                                  ✔ destination reached
 1  10.0.0.1        gw.local          0.4   0.3   0.4 ms
 2  100.64.0.1                        6.1   5.9   6.3 ms
 3  *               *                 *     *     *
```

## Options

| Field | Default | Meaning |
|---|---|---|
| **Max hops** | 30 | Give up after this many hops (1 to 64) |
| **Timeout** | 1000 ms | Per probe (100 ms to 10 s) |
| **Resolve names** | | Reverse DNS on each hop address |
| **Prefer IPv6** | | Use AAAA when a name has both |

Each hop gets **3 probes**, so you see variation per hop. The trace stops when the destination answers, or when a hop reports unreachable (the reason goes in the note column).

## Reading it

- **`*` on a middle hop, later hops fine**: that router just doesn't answer ICMP time-exceeded. Not a problem.
- **`*` from some hop to the end**: traffic stops there: firewall, ACL or routing.
- **Latency jumps at one hop and stays high**: the link at that hop. A jump on one hop that drops back later is that router de-prioritising ICMP, not real delay.
- **100.64.x.x hop**: carrier-grade NAT; the site has no public IPv4 of its own (port forwarding from outside won't work).

## Export

CSV columns: `host, ip, hop, address, name, rtt1_ms, rtt2_ms, rtt3_ms, note`.
