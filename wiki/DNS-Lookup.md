```
Name or IP [contoso.com] [Start]  Type [MX ⏷]  Resolver [Compare all ⏷]
```

## Type

Any single record type (A/AAAA together, MX, TXT, CNAME, NS, SOA, SRV, CAA, PTR, DS, DNSKEY), or **All** to ask for every one at once. An IP address is always a reverse (PTR) lookup.

## Resolver

| Resolver | Asks |
|---|---|
| **System** | This machine's configured DNS (sees hosts file, split DNS, the DC) |
| **Cloudflare** | 1.1.1.1 |
| **Google** | 8.8.8.8 |
| **Quad9** | 9.9.9.9 |
| **Authoritative** | The domain's own nameserver, bypassing every cache |
| **Compare all** | All of the above side by side, flagging answers that differ |

Public and authoritative queries go straight to that server (UDP, TCP when truncated), not through the OS resolver, so they show what the internet sees.

## When to use Compare all

| Symptom | Likely cause Compare shows |
|---|---|
| Works inside the office, not outside (or the reverse) | **Split DNS**: System answers differ from public |
| A change "isn't live yet" | **Stale cache**: Authoritative has the new value, a public resolver still has the old one |
| One user can't reach a site | Hosts file or local DNS override: only System differs |

## Export

CSV columns: `lookup, source, type, value` (Compare notes appear as type `note`).

For a full mail and DNS audit of a domain, use [Domain Health](Domain-Health).
