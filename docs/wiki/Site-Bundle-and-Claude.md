**Site Bundle…** (in the site menu, top left, any tab) gathers what the other tabs have produced into one `<site>.bundle.zip` to attach in Claude:

| File | From | For |
|---|---|---|
| `manifest.json` | site name, known problems, client handling, this machine's LAN/gateway/WAN, what each file is | both skills; its `mapnetwork_preflight` answers /mapnetwork's pre-build question |
| `prompt.txt` | the kickoff prompt, also copied to the clipboard on save | you, or paste it into Claude |
| `unifi.unifidump.json` | the UniFi backup open in Config Export, converted again with the bundle's client setting; a second is `unifi-2.unifidump.json`, and so on | /unifidump |
| `sonicwall.json` | the SonicWall export open in Config Export; a second is `sonicwall-2.json`, and so on | /mapnetwork (firewall config) |
| `meraki.json` | the Meraki export open in Config Export; a second is `meraki-2.json`, and so on | /mapnetwork (firewall config) |
| `inventory.json` | the last finished Subnet Sweep | /mapnetwork (scan) |
| `results.json` | the Results log, off by default: it holds every run since the app opened, which can include other clients' hosts and domains. The row lists the targets it covers so you can check before ticking it | /mapnetwork |
| `map.dc.html` | the `/mapnetwork` map open in Config Export, from an earlier visit; the prompt asks for it to be updated rather than rebuilt, under its original name | /mapnetwork (update) |
| `notes.md` | the Notes tab, off by default, sanitized (the row says how many values were redacted) | /mapnetwork (context and findings) |
| `skills/` | **Claude Skills**, on by default: the `/unifidump` and `/mapnetwork` skills themselves (about 650 KB) | a Claude chat that doesn't have the skills installed: the prompt tells it to follow `skills/<name>/SKILL.md` |

**UniFi clients** defaults to *anonymized* here: each client keeps its MAC, vendor and switch port (what unifidump needs to spot unadopted gear plugged in as a client) but device names, often people's names, become a token like `client-1a2b3c4d`, stable per MAC. *None* and *full* (real device names) are the other choices; client notes, email addresses and phone numbers are dropped in every mode. Saving also copies a kickoff prompt to the clipboard ("Run /unifidump on …, then /mapnetwork …"), so it's one attach and one paste: start a chat in Claude, attach the zip and paste. The current map alone is enough to save a bundle, for Claude to review and update it. Everything in the bundle comes from the sanitized exports; raw backups and secrets never go in.

**Several sites or firewalls:** every UniFi backup, SonicWall export and Meraki export open in Config Export goes in as a file of its own, in the order opened: the first under the usual name, the next numbered (`unifi-2.unifidump.json`, `sonicwall-2.json`, `meraki-2.json`). Each has its own row under **Includes**, named after its file and ending with the name it goes in as; untick one to leave it out, and the numbers close up. `manifest.json` gives each file's `source`, `firewall_config` lists every firewall file, and the prompt has Claude run `/unifidump` on each UniFi export and treat each as a separate controller, and each firewall file as a separate firewall with its own gateway.

## The kickoff prompt

`prompt.txt` (also on the clipboard after saving) is written for exactly what's in the zip. It tells Claude to read `manifest.json` first, to run `/unifidump` on each UniFi export if there is one, then `/mapnetwork` with everything else, and, when an earlier map is included, to update that map under its original name rather than start a new one. The manifest's `mapnetwork_preflight` answers `/mapnetwork`'s pre-build question (site name, scan, firewall config, known problems, existing map, clients on the map or not), so Claude doesn't ask it again.

Then open the `.dc.html` Claude makes on the Map (**⋯ › Open…**, or drop it there) or in Config Export; the bundle zip itself isn't read back (it says so if opened). See [Map → the /mapnetwork round trip](https://github.com/hammerknife404/is-alive/blob/main/Map#the-mapnetwork-round-trip).
