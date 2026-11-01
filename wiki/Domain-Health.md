The **Advanced** tab.

Enter a domain (`example.com`, `user@example.com` or a URL all work) and press Enter. The checks run in parallel against a public resolver queried directly (default `1.1.1.1`, any IP accepted) rather than the system one, so you see what the internet sees, not hosts-file or split-DNS answers. Each check reports PASS / WARN / FAIL / INFO with the reason and the raw records; checks with warnings or failures open automatically. **Stop** ends a run immediately, and results go to Copy, Save and the Results log like every other tab.

| Check | Validates |
|---|---|
| **MX** | missing, IP literal, CNAME target, host doesn't resolve, null MX (RFC 7505); detects Microsoft 365 / Google |
| **SPF** | missing or multiple records; DNS lookups against the 10 limit, counting every evaluation (an include reached twice counts twice); include loops; void lookups (`a`/`mx`/`exists` targets with no records, limit 2); `+all` / `?all` / no `all`, following `redirect=` to the record that holds it; `ptr`; includes with no SPF |
| **DMARC** | missing or duplicate, `p=none`, `sp=none`, no `rua=`, `pct<100`, external report domains not authorized; falls back to the organizational domain |
| **DKIM** | probes `selector1/2` (M365), `google`, `k1-3` (Mailchimp), `s1/s2` (SendGrid), `default`, … plus any extra selectors you enter; key size (<1024 fail, <2048 warn), naming the provider when the key sits behind its CNAME; revoked keys, warning when no active key is found; M365-style CNAMEs with no key behind them; a selector that fails to answer is reported without losing the others |
| **Nameservers** | fewer than 2 NS; each NS queried directly (no answer, lame/non-authoritative, serial drift); registrar/parent delegation vs the zone's NS set |
| **SMTP (port 25)** | connects to the first 3 MX hosts: 220 greeting, STARTTLS, TLS version, certificate validity and expiry, and an open-relay test (stops at RCPT, never sends mail). If every MX times out, reports that this network probably blocks outbound port 25 |
| **Blacklists** | self-hosted MX IPs, single `ip4:` SPF addresses and this network's WAN IP against Spamhaus ZEN, Barracuda, SpamCop, PSBL, Mailspike, UCEPROTECT L1, NiX Spam and DroneBL; the domain against Spamhaus DBL, URIBL and SURBL. Spamhaus PBL (end-user ranges) is INFO unless SPF authorizes the IP. Lists that refuse public resolvers are retried via this machine's DNS server |
| **MTA-STS** | TXT plus HTTPS policy: mode, max_age, every MX covered; says plainly when `mta-sts.<domain>` doesn't resolve |
| **TLS-RPT** | present, valid `rua=` |
| **BIMI** | HTTPS SVG logo (query string allowed), VMC/CMC, DMARC at quarantine/reject with pct=100 |
| **DNSSEC** | DS/DNSKEY, signed but no DS, bogus (SERVFAIL that clears with CD), resolver-validated (AD), deprecated algorithms / SHA-1 DS |
| **CAA** | allowed CAs (inherited from parent domains), critical unknown tags |
| **MX reverse DNS** | PTR on every MX address, forward-confirmed. WARN for self-hosted MX (it usually sends mail too); INFO for hosted services (M365, Google, Cloudflare, Mimecast, Proofpoint, Barracuda), whose inbound relays don't send your mail |
| **Autodiscover** | CNAME to `autodiscover.outlook.com` when mail is on Microsoft 365 |
| **Domain expiry** | RDAP via the IANA bootstrap (fetched once per session): expiry date, registrar, hold / pending-delete status |

DKIM keys can't be listed, only probed. If none of the common selectors match, take the `s=` value from a received message's `DKIM-Signature` header and add it in the **DKIM** field (Advanced's second row, beside **Resolver**).

MTA-STS and RDAP use HTTPS with the OS certificate store (works behind TLS-inspecting firewalls) and honor `HTTPS_PROXY` / the Windows system proxy.
