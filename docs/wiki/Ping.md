ICMP (IPv4/IPv6) or TCP-connect ping, continuous or N count, live latency graph, loss/min/avg/max.

Like every tab: the run bar (the main field first, **▶ Start** right after it), the other settings folded under **Options** with their values shown as chips, a status row under it with what the run found on the left and **Copy** / **Save…** on the right, and a short explanation of what to do before the first run. Enter in the main field starts a run; Esc stops it.

**ICMP** works as a normal user on both systems: Windows uses the IP Helper API (`IcmpSendEcho2` / `Icmp6SendEcho2`), Linux uses unprivileged ping sockets (`SOCK_DGRAM` / `IPPROTO_ICMP`), which need your group inside `net.ipv4.ping_group_range` (the default on Fedora, Ubuntu, Arch and most systemd distros; otherwise it falls back to the system `ping`). **TCP** times a connection to a port instead, for hosts that drop ICMP.

**CSV columns** (Results → Export → CSV): target, mode, port, seq, from, status, rtt_ms, ttl, bytes. See [[Results]].
