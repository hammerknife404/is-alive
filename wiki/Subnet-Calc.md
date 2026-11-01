Offline IPv4 / IPv6 calculator. Nothing leaves the machine. It opens on this machine's own subnet (**This network** puts it back).

## Inputs it understands

| Type | Example | Shows |
|---|---|---|
| CIDR | `10.0.0.0/22`, `2001:db8::/48` | network, broadcast, host range, counts, mask in binary, address type, neighbouring blocks |
| Address + netmask | `10.0.0.5 255.255.252.0` | the block it sits in |
| Wildcard (ACL) mask | `10.0.0.0 0.0.3.255` | same, from a Cisco/SonicWall-style wildcard |
| Range | `10.0.0.5-10.0.1.20` | the fewest CIDR blocks that exactly cover it |

The address type names private (RFC 1918), CGNAT (100.64/10), link-local, loopback, multicast, documentation and so on.

## Actions

- **Check address**: enter an address to check whether it falls inside the block.
- **Split into**: pick a longer prefix to list the smaller blocks (up to 65,536 rows, e.g. a /16 into /32s).
- **Sweep**: sends the block (IPv4, up to 65,536 hosts) to [Subnet Sweep](Subnet-Sweep).

## Example (illustrative)

```
10.0.0.0/22
Network    10.0.0.0         Broadcast  10.0.3.255
Hosts      10.0.0.1 – 10.0.3.254    (1,022 usable)
Mask       255.255.252.0   11111111.11111111.11111100.00000000
Wildcard   0.0.3.255
Type       Private (RFC 1918)
Neighbours 10.0.4.0/22 ▸   ◂ 9.255.252.0/22
```
