**Claude Handoff** (in the nav under **VISIT**, or **Claude Handoff…** in the site menu, any tab) gathers what the other tabs have produced into one `<site>.bundle.zip` to attach in Claude. (It was called the Site Bundle until 0.30.5; the zip, its manifest and the skills keep their names.)

It's a five-step wizard, written so a partner or a first-line tech can do it without this page: each step says what it's for, the steps across the top can be clicked in any order, and any can be skipped.

| Step | What's there |
|---|---|
| **1 Site** | How the handoff works in four lines; **Site name** (filled from a UniFi backup's site); **Known problems (optional)**, with **Redact** |
| **2 Configs** | **Open files…** for UniFi backups, SonicWall and Meraki exports and last visit's `.dc.html` (they open in Config Export and on the Map too); a row per file; **Where to get them** for each kind |
| **3 Sweep** | **Subnets or ranges**, filled with this LAN, and **Sweep** right there (tell the site's IT contact first); the **Sweep inventory** and **Results log** rows |
| **4 Notes** | **Tech notes** and **Card notes**, with what was redacted, and an amber line when one still looks like it holds a password, an email or a patient's details (only what you marked is redacted); **Client names** (what becomes of client devices' names), and whether the map draws them |
| **5 Save** | Every row to check; **Save for Claude…** saves the zip and copies the prompt, then lists what to do in Claude using the zip's real name; **Copy the prompt again** |

What goes in the zip:

| File | From | For |
|---|---|---|
| `manifest.json` | site name, known problems, client handling, this machine's LAN/gateway/WAN, what each file is | both skills; its `mapnetwork_preflight` answers /mapnetwork's pre-build question |
| `prompt.txt` | the kickoff prompt, also copied to the clipboard on save | you, or paste it into Claude |
| `unifi.unifidump.json` | the UniFi backup open in Config Export, converted again with the bundle's client setting; a second is `unifi-2.unifidump.json`, and so on | /unifidump |
| `sonicwall.json` | the SonicWall export open in Config Export; a second is `sonicwall-2.json`, and so on | /mapnetwork (firewall config) |
| `meraki.json` | the Meraki export open in Config Export; a second is `meraki-2.json`, and so on | /mapnetwork (firewall config) |
| `inventory.json` | the last finished Subnet Sweep, phones', laptops' and PCs' names as **Client names** says (its `client_names`) | /mapnetwork (scan) |
| `results.json` | the Results log, off by default: it holds every run since the app opened, which can include other clients' hosts and domains. The row lists the targets it covers so you can check before ticking it | /mapnetwork |
| `map.dc.html` | the `/mapnetwork` map open in Config Export, from an earlier visit; the prompt asks for it to be updated rather than rebuilt, under its original name | /mapnetwork (update) |
| `notes.md` | the Notes tab, off by default, redacted as Notes redacts it: what you marked and the loaded configs' secrets, nothing else (the row says how many values were redacted, and warns when something still looks secret) | /mapnetwork (context and findings) |
| `skills/` | **Claude Skills**, on by default: the `/unifidump` and `/mapnetwork` skills themselves (about 650 KB) | a Claude chat that doesn't have the skills installed: the prompt tells it to follow `skills/<name>/SKILL.md` |

**Client names** (step 4) defaults to *anonymized* here: each UniFi client keeps its MAC, vendor and switch port (what unifidump needs to spot unadopted gear plugged in as a client) but device names, often people's names, become a token like `client-1a2b3c4d`, stable per MAC. The same goes for the names of phones, laptops and PCs in the sweep inventory and in Meraki DHCP reservations: anything the Map wouldn't call network gear, a NAS, a printer or a camera, the gateway aside. A device gets the same token in every file, so Claude can still match them. *None* (UniFi client table left out, those names left out) and *full* (real device names) are the other choices; client notes, email addresses and phone numbers are dropped in every mode. The Sweep's own **Inventory…** button saves every name, as the table shows them: it's your copy. Saving also copies a kickoff prompt to the clipboard ("Run /unifidump on …, then /mapnetwork …"), so it's one attach and one paste: start a chat in Claude, attach the zip and paste. The current map alone is enough to save a handoff, for Claude to review and update it. Everything in the bundle comes from the sanitized exports; raw backups and secrets never go in.

**Several sites or firewalls:** every UniFi backup, SonicWall export and Meraki export open in Config Export goes in as a file of its own, in the order opened: the first under the usual name, the next numbered (`unifi-2.unifidump.json`, `sonicwall-2.json`, `meraki-2.json`). Each has its own row in steps 2 and 5, named after its file and ending with the name it goes in as; untick one to leave it out, and the numbers close up. `manifest.json` gives each file's `source`, `firewall_config` lists every firewall file, and the prompt has Claude run `/unifidump` on each UniFi export and treat each as a separate controller, and each firewall file as a separate firewall with its own gateway.

## The kickoff prompt

`prompt.txt` (also on the clipboard after saving) is written for exactly what's in the zip. It tells Claude to read `manifest.json` first, to run `/unifidump` on each UniFi export if there is one, then `/mapnetwork` with everything else, and, when an earlier map is included, to update that map under its original name rather than start a new one. The manifest's `mapnetwork_preflight` answers `/mapnetwork`'s pre-build question (site name, scan, firewall config, known problems, existing map, clients on the map or not), so Claude doesn't ask it again.

Then open the `.dc.html` Claude makes on the Map (**More › Open…**, or drop it there) or in Config Export; the zip itself isn't read back (it says so if opened). See [Map → the /mapnetwork round trip](https://github.com/hammerknife404/is-alive/blob/main/Map#the-mapnetwork-round-trip).

**No map from Claude yet?** The Map's **Export › /mapnetwork map (.dc.html)** saves the map is-alive drew as a `/mapnetwork` page. Open it in step 2 like one Claude made, and Claude builds on it rather than starting from nothing.
