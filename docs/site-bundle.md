# Site Bundle

**Site Bundle…** (top right, any tab) gathers what the other tabs have produced into one `<site>.bundle.zip` to attach in Claude:

| File | From | For |
|---|---|---|
| `manifest.json` | site name, known problems, client handling, this machine's LAN/gateway/WAN, what each file is | both skills; its `mapnetwork_preflight` answers /mapnetwork's pre-build question |
| `prompt.txt` | the kickoff prompt, also copied to the clipboard on save | you, or paste it into Claude |
| `unifi.unifidump.json` | the UniFi backup open in Config Export, converted again with the bundle's client setting | /unifidump |
| `sonicwall.json` | the SonicWall export open in Config Export | /mapnetwork (firewall config) |
| `meraki.json` | the Meraki export open in Config Export | /mapnetwork (firewall config) |
| `inventory.json` | the last finished Subnet Sweep | /mapnetwork (scan) |
| `results.json` | the Results log, off by default: it holds every run since the app opened, which can include other clients' hosts and domains. The row lists the targets it covers so you can check before ticking it | /mapnetwork |
| `map.dc.html` | the `/mapnetwork` map open in Config Export, from an earlier visit; the prompt asks for it to be updated rather than rebuilt, under its original name | /mapnetwork (update) |
| `notes.md` | the Notes tab, off by default, sanitized (the row says how many values were redacted) | /mapnetwork (context and findings) |

**UniFi clients** defaults to *anonymized* here: each client keeps its MAC, vendor and switch port (what unifidump needs to spot unadopted gear plugged in as a client) but device names, often people's names, become a token like `client-1a2b3c4d`, stable per MAC. *None* and *full* (real device names) are the other choices; client notes, email addresses and phone numbers are dropped in every mode. Saving also copies a kickoff prompt to the clipboard ("Run /unifidump on …, then /mapnetwork …"), so it's one attach and one paste. Everything in the bundle comes from the sanitized exports; raw backups and secrets never go in.
