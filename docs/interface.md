# Interface, Results and export

## Settings

Click **is-alive** at the top left. It holds the theme, the Miami backdrop, the Map's accuracy note, keyboard and mouse tips, and the folder where settings and notes are kept.

## Themes

The theme list in **Settings** offers Dark (the default: egui's stock dark look), Light, Miami, and themes after `/mapnetwork`'s: Slate, Phosphor, Blueprint, High contrast, RGB (neon on black with a rainbow line under the tabs) and Paper. Each theme has its own Map colours (the menu shows them as swatches beside each name), and cards and links are tinted in their role's colour, with a soft glow on dark themes. The choice is remembered between launches (`%APPDATA%\is-alive\settings` on Windows, `~/.config/is-alive/settings` on Linux). Miami puts the panels over a backdrop: a 16-bit synthwave scene drawn by the app (sunset, skyline, palms, a grid drifting toward you), your own picture with a slow pan (only its path is saved; the image never leaves your machine), or none. **Animate** loops the scene and the sunset line under the tabs and pauses while the window isn't focused; turn it off on battery. **Glow** adds a soft halo and haze to the bright parts (the sun, neon and windows on the GPU; your picture's highlights either way). **VHS** (on by default, GPU only) plays the scene as if off a worn tape: faint scanlines, red and blue bleeding a hair apart, grain, lines jittering sideways, and every 9 seconds a tracking band rolling down the picture. The scene, a picture's pan and the sunset line run at 10 frames a second, a slight chop that suits the look of a 16-bit scene off a VHS tape and keeps it cheap (the whole app redraws every frame; at the display's rate the Map tab took a third of a CPU core). On the GPU the scene is drawn at art resolution by a shader into an offscreen buffer and scaled up with hard edges, and a repaint between frames (a hover, typing) reuses that buffer. Without a usable GPU (a software OpenGL such as llvmpipe or Microsoft's Basic Render Driver, OpenGL below 3.1 / ES 3.0, or a shader the driver rejects) it's drawn on the CPU instead. Settings says which is in use and why; `IS_ALIVE_GPU=off` forces the CPU.

## Opening files and shortcuts

Files named on the command line (or opened with **Open with → is-alive**) go straight to Config Export, as if dropped on the window.

**Ctrl+1 … Ctrl+9** switch to the first nine tabs and **Ctrl+0** to Results (Settings → **Keyboard and mouse** lists every shortcut; Speed Test and Subnet Calc, just before Results, have none), copies and saves confirm with a toast, and the window opens at the size it was closed at.

## Results

Every result view has a **Copy** button for pasting into tickets. The **Results** tab keeps the 12 newest runs open and folds older ones, **Clear** asks for a second click, and it has a **Filter** box (tool name or output text) and **Export** to CSV, JSON, Markdown or HTML, for the whole log or a single run; Copy, Save and Export all use the filtered set.

## CSV export

CSV export uses each tool's own columns, one row per result, with the run's start time first, so repeated runs stack and filter cleanly in Excel:

| Tool | Columns |
|---|---|
| Ping | target, mode, port, seq, from, status, rtt_ms, ttl, bytes |
| Port Check | host, ip, port, state, service, rtt_ms, error (closed/filtered included) |
| Traceroute | host, ip, hop, address, name, rtt1_ms, rtt2_ms, rtt3_ms, note |
| Speed Test | server, ip, phase (idle/download/upload), mbps, mb, seconds, streams, probe, probes, lost, latency_avg_ms, latency_min_ms, latency_max_ms, jitter_ms, loss_pct, added_ms, mos |
| Subnet Sweep | range, ip, rtt_ms, ttl, mac, vendor, name, open_ports, found_by (alive hosts; `arp` rows have no rtt_ms) |
| DNS Lookup | lookup, source, type, value (Compare notes as type `note`) |
| Advanced | domain, resolver, section, check, check_status, kind (finding/record), status, text |
| Host info | field, value |

Exporting the whole log gives one CSV: a `tool` column plus every tool's columns (shared ones like `ip` and `rtt_ms` line up), so filter on `tool` in Excel. Runs that failed before producing results carry their text in an `output` column.
