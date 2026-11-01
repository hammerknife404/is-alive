| Symptom | Cause | Fix |
|---|---|---|
| *"Windows protected your PC"* on first launch | The `.exe` isn't code-signed | **More info → Run anyway**. Verify the `.sha256` first if in doubt ([Getting Started](Getting-Started)) |
| EDR (CrowdStrike, Defender) alerts on Port Check or Sweep | Any connect or ICMP sweep is a scan | Use **Gentle** speed, and exclude the tool or the tech's host for sanctioned use ([Scan Speed](Scan-Speed)) |
| AppImage won't start: `GLIBC_2.35 not found` | Built on Ubuntu 22.04 (glibc 2.35) | Use a distro at least that new, or [build it](Building-and-Releasing) on the older host |
| Linux: app uses the system `ping` | Your group isn't in `net.ipv4.ping_group_range` | `sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"` (persist in `/etc/sysctl.d/`), or ignore: the fallback works |
| Sweep finds no *ARP only* devices on Linux | `ip` (iproute2) missing | Install `iproute2`; without it the sweep is ping-only |
| Sweep shows no MAC / vendor for some hosts | They're past a router: ARP only works on your own segment | Sweep from a machine on that VLAN, or load the UniFi backup |
| Every port **filtered** on a host that's up | Host firewall drops SYNs, or you're testing UDP services | Check from the same subnet; UDP (IKE 500/4500, SNMP) can't be tested at all |
| Domain Health: SMTP times out on every MX | This network blocks outbound 25 (common on ISP and guest networks) | The report says so. Test from another network |
| Speed Test fails in one direction | Web filter (SonicWall CFS, DNS filtering) blocks speed test sites, or Cloudflare rate-limits | Try **Download a file** with a preset server, or **Upload to a URL** |
| Speed Test notes it ran over IPv4 only | The network hands out IPv6 but can't route it | Not the app: a real misconfig worth a ticket. The test continues on IPv4 |
| Speed Test latency switched to TCP 443 | ICMP to the server is blocked | Fine, but loss is less reliable that way |
| Miami theme is slow / Settings says *CPU* | No usable GPU (llvmpipe, Basic Render Driver, OpenGL < 3.1) | Turn **Animate** off, or pick another theme. `IS_ALIVE_GPU=off` forces CPU deliberately |
| Config Export refuses a file | Over 2 GB, nested too deep, or not a format it knows | Check it's the right file (`.unf`/`.unifi`/`.exp`/`.wri`/Meraki `.json`/`.dc.html`) and not truncated |
| MTA-STS / RDAP fail behind a proxy | — | They honour `HTTPS_PROXY` and the Windows system proxy, and use the OS cert store (TLS inspection is fine) |

Still stuck? Open an [issue](https://github.com/hammerknife404/is-alive/issues) with the **Copy** output of the failing tab (it's sanitized, but check it first).
