## Download

From the [latest release](https://github.com/hammerknife404/is-alive/releases/latest), under **Assets**:

| OS | File | First run |
|---|---|---|
| Windows 10/11 | `is-alive.exe` | Double-click. On *"Windows protected your PC"* click **More info → Run anyway** (unsigned binary). |
| Linux (x86_64) | `is-alive-x86_64.AppImage` | `chmod +x is-alive-x86_64.AppImage && ./is-alive-x86_64.AppImage` |

Each binary has a `.sha256` beside it:

```powershell
Get-FileHash .\is-alive.exe -Algorithm SHA256   # compare with is-alive.exe.sha256
```
```sh
sha256sum -c is-alive-x86_64.AppImage.sha256
```

No installer, no admin rights. Run it from a USB stick or the Downloads folder.

## First run

1. The window opens on **Ping**. Type a host and press **Enter**.
2. Tabs across the top are the tools; **Ctrl+1 … Ctrl+9** jump to the first nine, **Ctrl+0** to Results.
3. Every result has **Copy** (paste into a ticket) and **Save**. Every run also lands in the **Results** tab for export.
4. Click **is-alive** at the top left for Settings: theme, keyboard and mouse tips, and where settings are kept.

## Where things are stored

| | Windows | Linux |
|---|---|---|
| Settings | `%APPDATA%\is-alive\settings` | `~/.config/is-alive/settings` |
| Notes | `notes.md` in the same folder | same |

The Results log is memory only and is gone when the app closes. See [Privacy and Security](Privacy-and-Security).

## Opening files

Drag a UniFi backup, SonicWall export or report, Meraki JSON or `/mapnetwork` map onto the window from any tab, pass it on the command line, or use **Open with → is-alive**. It goes to [Config Export](Config-Export).

```sh
./is-alive-x86_64.AppImage site.unf firewall.exp
```

## Linux: ICMP without root

The AppImage uses unprivileged ping sockets. That works when your group is inside `net.ipv4.ping_group_range`, the default on Fedora, Ubuntu, Arch and most systemd distros:

```sh
sysctl net.ipv4.ping_group_range    # "0 2147483647" = everyone
```

If it isn't, the app falls back to the system `ping` command automatically.
