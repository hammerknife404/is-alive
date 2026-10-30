# is-alive demo: follow-along guide

This guide is for anyone attending the is-alive demo. You don't need any technical background. Follow the steps and match what you see with the presenter's screen.

**is-alive** is a small program that checks whether a network is healthy: whether the internet connection works, whether websites can be found, and whether a company's email is set up safely. It's one file, needs no installation, and doesn't need administrator rights.

---

## 1. Get the app (2 minutes)

1. Open the [latest release page](https://github.com/hammerknife404/is-alive/releases/latest).
2. Under **Assets**, download the file for your computer:
   - **Windows:** `is-alive.exe`
   - **Linux:** `is-alive-x86_64.AppImage`
3. Open it:
   - **Windows:** double-click `is-alive.exe`. If a blue box says *"Windows protected your PC"*, click **More info**, then **Run anyway**. Windows shows this for any new program that isn't from a large publisher.
   - **Linux:** right-click the file → **Properties** → **Permissions** → tick **Allow executing file as program**, then double-click it.
4. Check the version: the title bar at the top of the window says `is-alive 0.7.2` (or newer).

You'll see a row of tabs across the top: **Ping**, **Port Check**, **Traceroute**, **Subnet Sweep**, **DNS Lookup**, **Advanced**, **UniFi Backup** and **Results**. The bar at the bottom shows your computer's local address, your router (gateway) and your public internet address.

> **Please watch, don't run, the Port Check and Subnet Sweep parts.** They look at other devices on the network, and the company's security software may flag that as suspicious. Everything else in this guide is safe to run yourself.

---

## 2. One-click health check (watch along)

The presenter clicks the **is-alive** button at the top left. Several checks start at once and the app jumps to the **Results** tab.

| Result | What it tells you |
|---|---|
| **Host info** | Your computer's address, your router's address and your public internet address |
| **Ping** | Your router answers, and how fast (in milliseconds; lower is better) |
| **Port Check** | Which services the router offers (for example, web pages on 443) |
| **Traceroute** | Every stop your traffic makes on the way to the internet |
| **DNS Lookup** | Website names are being turned into addresses correctly |
| **Subnet Sweep** | Every device currently on the local network |

**Takeaway:** in about 15 seconds you know whether the problem is your computer, your router, or the internet beyond it.

---

## 3. Try it yourself: Ping

1. Click the **Ping** tab.
2. In **Host**, type `1.1.1.1` and press **Enter**.
3. Watch the line graph: each point is one reply. A flat, low line is good. Spikes mean delays; red marks mean a reply never came back.
4. Click **■ Stop** when you've seen enough.

**What it means:** "ping" is like knocking on a door and timing how long it takes someone to answer.

---

## 4. Try it yourself: DNS Lookup

DNS is the internet's phone book: it turns names like `gmail.com` into the numeric addresses computers use.

1. Click the **DNS Lookup** tab.
2. Type `gmail.com` in **Name or IP**.
3. Set **Type** to **MX** and **Resolver** to **Cloudflare 1.1.1.1**, then press **Enter**.
4. You'll see five mail servers. MX records tell the world where to deliver email for a domain.
5. Now try the "compare" view: type `localhost`, set Type to **A + AAAA**, set Resolver to **Compare all**, press **Enter**.
6. A yellow warning appears saying the answers differ. That's on purpose here: `localhost` means "this computer", so your machine answers differently from the internet. With a real company domain, a warning like this means people inside and outside the office see different things, a common cause of "it works for me but not for them".

---

## 5. Try it yourself: email health check (Advanced)

This tab checks a company's email security settings, the same ones that decide whether its emails land in the inbox or the spam folder.

1. Click the **Advanced** tab.
2. Type `gmail.com` in **Domain** and press **Enter**.
3. About 15 checks appear within a second or two. Each has a label:

| Label | Colour | Meaning |
|---|---|---|
| **PASS** | green | Set up correctly |
| **WARN** | yellow | Works, but could be safer or more reliable |
| **FAIL** | red | Broken or missing; worth fixing |
| **INFO** | grey | For information only, not a problem |

4. Click any row to open it. Each one explains in plain words what was found and shows the record it's based on.

Things to notice on `gmail.com`:
- **DMARC: WARN.** Gmail's own policy is set to "monitor only", so the check correctly flags it.
- **SMTP (port 25): INFO.** On many office and home networks this check can't connect, because the internet provider blocks that kind of traffic. The app says so instead of reporting a false failure.

Then try these, one at a time:

| Type this | What you should see |
|---|---|
| `cloudflare.com` | **MTA-STS: FAIL.** Even a big company can have a broken setting: they published a security policy that can't actually be fetched. |
| `dbltest.com` | **Blacklists: FAIL.** This is a test address that spam blocklists deliberately list, so it shows what a blacklisted domain looks like. (If it says *couldn't check* instead, this network uses a public DNS service the blocklist refuses to answer. That's fine.) |
| `example.com` | Mostly PASS and INFO. This domain is set up to say "I never send email", and the app recognises that. |

**Takeaway:** the app doesn't just check that settings exist, it checks they actually work.

---

## 6. Results and sharing

1. Click the **Results** tab. Every check you ran is listed with its start and end time.
2. **Copy** puts everything on the clipboard, ready to paste into an email or support ticket. **Save** writes it to a text file.

---

## Glossary

| Term | Plain meaning |
|---|---|
| **IP address** | A device's number on a network, like a phone number (e.g. `1.1.1.1`) |
| **Gateway / router** | The box that connects your local network to the internet |
| **LAN** | The local network in a building |
| **WAN / public IP** | The single address the internet sees for your whole network |
| **Ping** | A quick "are you there?" message that measures response time |
| **ms (milliseconds)** | Thousandths of a second; under 50 ms feels instant |
| **DNS** | The internet's phone book: names to addresses |
| **MX record** | Tells the world which server receives a domain's email |
| **SPF** | A list of servers allowed to send email for a domain |
| **DKIM** | A digital signature on outgoing email proving it wasn't forged |
| **DMARC** | Tells receivers what to do with email that fails SPF/DKIM: nothing, spam folder, or reject |
| **Blacklist** | A shared list of senders known for spam; being on one sends your email to junk |
| **Port** | A numbered door on a device for one kind of service (443 = secure websites, 25 = email) |

---

## Questions after the demo?

Ask the presenter, or open the app again later: everything in this guide works the same on your own.
