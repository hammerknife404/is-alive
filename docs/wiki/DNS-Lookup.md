Any record type, or **All** at once (A/AAAA, MX, TXT, CNAME, NS, SOA, SRV, CAA, PTR, DS, DNSKEY) via the system resolver, Cloudflare, Google, Quad9 or the domain's authoritative server, or **Compare all** side by side to spot split DNS and stale caches.

Like every tab: the run bar (the main field first, **▶ Start** right after it, then **Type** and **Resolver**, which are the lookup itself, so Lookup folds nothing under **Options**), a status row under it with what the run found on the left and **Copy** / **Save…** on the right, and a short explanation of what to do before the first run. Enter in the main field starts a run; Esc stops it.

The line under the toolbar says what the chosen resolver answers: **System** as apps see it (hosts file, suffixes), **Authoritative** from the zone's own nameserver (what's published, before any caching), **Compare** every resolver and the authoritative server side by side, flagging answers that differ, or a public resolver asked directly.

**CSV columns** (Results → Export → CSV): lookup, source, type, value (Compare notes as type `note`). See [[Results]].
