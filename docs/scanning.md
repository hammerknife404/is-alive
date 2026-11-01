# Scanning: Subnet Sweep and scan speed

## Subnet Sweep: MAC, vendor and inventory

Each live host's **MAC** comes from this machine's ARP table (Windows `GetIpNetTable2`, Linux `/proc/net/arp`; no admin rights), so only hosts on your own segment have one. **Vendor** is the IEEE registry owner of the MAC prefix (MA-L, MA-M and MA-S, bundled offline; rebuild with `python3 ../scripts/update-oui.py`). A locally administered MAC shows as *private/virtual*: a phone or laptop's per-network private address, or a VM/container NIC.

**Ping-silent devices.** Pinging an address on your own segment makes the OS send an ARP request first, and almost nothing refuses ARP. Hosts that answer ARP but drop ping (Windows firewall defaults, many IoT devices and cameras) are listed with *no ping* in the Time column and counted as *ARP only* in the header. Only entries the OS has just confirmed count (Linux `ip neigh` REACHABLE, Windows Reachable), so devices that left minutes ago don't show up. The sweep reads the table every 3 s while it runs. Past a router there is no ARP, so only ping counts there. On Linux this needs `ip` (iproute2); without it the sweep is ping-only. The Inventory marks each host `found_by` `ping` or `arp`.

**Check mgmt ports** adds a TCP connect after the sweep to each live host on 22, 23, 80, 443, 445, 541 (FortiGate), 554 (RTSP), 3389, 4433 (SonicWall), 5000, 8080/8443 (UniFi), 8291 (MikroTik) and 9100 (printers), at the same speed limit. Open management ports are the evidence for telling switches, APs and firewalls from clients. It's off by default: it is a port scan.

**Inventory** saves the run as JSON for `/mapnetwork`'s scan input: this machine's LAN, gateway and WAN, the ports checked, and per host `ip`, `mac`, `vendor`, `private_mac`, `name`, `rtt_ms`, `ttl`, `open_ports`, `is_gateway` and `is_this_machine`. Facts only, no role guesses; the MACs are what join it to `/unifidump`'s `discovered_devices` and uplink MACs.

## Scan speed

Port Check and Subnet Sweep have a **Speed** preset. Threads only cap how many probes are in flight; the **rate** (probes started per second, evenly spaced) is what stops a scan from bursting against hosts that answer fast.

| Preset | Rate | In flight | 1-1024 ports | /24 sweep |
|---|---|---|---|---|
| Gentle | 10/s | 4 | ~1m 43s | ~26 s |
| Normal (default) | 50/s | 16 | ~21 s | ~5 s |
| Fast | 500/s | 64 | ~2 s | <1 s |
| Unthrottled | none | 256 | tens of ms | tens of ms |

Any TCP connect or ICMP sweep is a scan by definition; throttling reduces noise and load but won't make it invisible to IDS/EDR. For sanctioned use, exclude the tool or the source host in the security stack.

ICMP goes through the Windows IP Helper API (`IcmpSendEcho2` / `Icmp6SendEcho2`), so it works as a normal user — no raw sockets.
