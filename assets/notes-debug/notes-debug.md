# Notes debug checklist 🐛

Paste this whole file into the Notes editor (Ctrl+A in the editor, then paste), switch to **Split**, and work down the list. Every section says what to do and what you should see. The viewer files in this folder are for **Open…**; see the last section.

Drawing of every Markdown element on its own is in `assets/notes-format-test.md`; this file covers what the Notes tab *does*.

## 1. Views and layout

- [ ] **Edit** shows only the editor; **Preview** only the drawn note; **Split** both.
- [ ] In Split, drag the divider: it stops at 20% and 80%. Double-click it: back to half and half.
- [ ] Restart the app: the view, divider position, font and size come back as you left them.
- [ ] Resize the window narrow (about 620 px): the toolbar wraps, nothing is cut off.

## 2. Font and size (right of the toolbar)

- [ ] Each font in the font menu changes the editor and Preview. Sans and Mono are always there; Consolas / Cascadia Mono / JetBrains Mono / DejaVu Sans Mono etc. only where installed.
- [ ] Sizes 8 to 32 all apply; at 32 this line still wraps in Preview: The quick brown fox jumps over the lazy dog while the switch on port 24 renegotiates at 100 Mb half duplex again.
- [ ] Monospace alignment check (columns line up in a mono font, not in Sans):

```
Port  VLAN  Speed   Device
1     10    1000F   FW-01
24    20    100H    PRN-LOBBY
```

## 3. Toolbar formatting buttons

Select the word in each line, then press the button (or the shortcut). Undo with Ctrl+Z.

- [ ] Bold this: **B** or Ctrl+B → `**this**`
- [ ] Italic this: **I** or Ctrl+I → `*this*`
- [ ] Code this: **</>** → `` `this` ``
- [ ] Press Ctrl+B with nothing selected: a pair of `****` with the cursor in the middle.
- [ ] Cursor on this line, press **H**: it becomes a heading (`# `).
- [ ] Cursor on this line, press **•**: it becomes a bullet (`- `).
- [ ] Cursor on this line, press **☐**: it becomes a task (`- [ ] `).

## 4. Lists that continue (Enter)

Put the cursor at the end of each last item and press Enter.

- Bullet: Enter starts `- ` on the next line
* Asterisk bullet: Enter keeps `* `
  - Nested two spaces: Enter keeps the indent
- [ ] Task: Enter starts a new empty `- [ ] `
- [x] Ticked task: Enter starts an *unticked* one

1. Numbered
2. Enter here gives `3. `
9. Enter here gives `10. `

- [ ] Press Enter twice at the end of a list: the second Enter removes the empty marker and ends the list.
- [ ] Enter in the middle of a list item splits it and the new line gets the marker.
- [ ] Enter on a plain line (this one) adds nothing.

## 5. Task boxes in Preview

- [ ] Click this box in Preview: the source changes to `[x]`, the line is struck through and dimmed.
- [x] Click this one: back to `[ ]`.
- [X] Capital X counts as done too.
  - [ ] Nested task ticks on its own.

## 6. Emoji

- [ ] The emoji button opens the picker; it stays open while you click several; each goes in at the cursor.
- [ ] Hovering one in the picker shows what it's for and its shortcode.
- [ ] Switch style at the top of the menu (Match theme / Monochrome / Colour / Miami neon) and look at this row: ✅ ❌ ⚠ 🔥 💡 📌 🔑 🖥 📶 🐛
- [ ] Colour: ✅ green, ❌ red, faces yellow 😀. Match theme in Miami or RGB: neon.
- [ ] Shortcodes in Preview become emoji: :white_check_mark: :x: :warning: :fire: :bulb: :key: :+1: :-1: :bug: :construction: :sparkles:
- [ ] Not shortcodes, left alone: 10:30, :nope:, a lone colon : here, http://example.com:8080
- [ ] Emoji are a size up but don't push the line height around: 🔥 text 🔥 text 🔥

## 7. Time and Host info

- [ ] **Time** inserts the date, time, time zone and where the clock syncs from (w32tm on Windows; chrony or timesyncd on Linux), at the cursor.
- [ ] **Host info** inserts computer name (and AD domain on Windows), LAN, MAC, gateway and WAN. WAN shows "not looked up" until Refresh in the footer has run.

Insert both here:



## 8. Redaction by hand (Ctrl+R)

Select each value below (just the value) and press Ctrl+R, or **Redact** in the toolbar.

- Wi-Fi key: Harbor-Guest-2026!
- Local admin: Sup3r$ecretPass
- Same key again, should go too: Harbor-Guest-2026!

Expected:
- [ ] Both copies of `Harbor-Guest-2026!` are highlighted in the editor (it redacts everywhere it appears).
- [ ] Preview shows `[🔑 REDACTED: …]` markers in the warning colour on a tint; the editor keeps the real text, highlighted.
- [ ] The footer says what's redacted (🔑 …); clicking it highlights them in the editor.
- [ ] **By hand (n)** in the footer lists them as the first two characters and a length (`Ha… (18 chars)`), never in full; × stops redacting one.
- [ ] Select a redacted value and press Ctrl+R again (the button now says **Unredact**): it shows again everywhere and is listed under "Kept as written".
- [ ] Restart the app: the redaction list is gone (memory only) but the saved note holds just the marker, not the secret.

Already-redacted markers (drawn as markers, not plain text):
- [🔑 REDACTED: password]
- [REDACTED: secret]
- [REDACTED with no colon: plain text]

## 9. Redaction from a loaded config

- [ ] Load a UniFi backup or SonicWall export in Config Export, then type one of its secrets here (a Wi-Fi passphrase or RADIUS secret from that config):

    paste-a-secret-from-the-config-here

- [ ] It's highlighted and counted as "from the config" in the footer without pressing Ctrl+R.
- [ ] Pattern rules (keys, card numbers, emails) are *not* applied to your own notes; these stay as written:
  - admin@example.com, 4111 1111 1111 1111, -----BEGIN PRIVATE KEY-----

## 10. What goes out

- [ ] **Copy** copies the redacted text (markers in place of secrets). Paste somewhere to check.
- [ ] **Raw copy** asks for a second click, then copies the real text.
- [ ] **Save .md…** saves the redacted text; open the file to check.
- [ ] **Site Bundle…** includes the redacted notes.
- [ ] **Clear** asks for a second click, then empties the notes.
- [ ] The footer word count changes as you type; "Autosaved hh:mm" appears a moment after you stop.
- [ ] Close the app mid-typing and reopen: nothing lost (it saves on exit too).

## 11. Size and odd text

- [ ] Long unbroken line wraps in Preview, scrolls in Edit: https://example.com/a/very/long/path/that/never/breaks/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
- [ ] Tabs:	between	these	words (a tab character each)
- [ ] Right-to-left and combining text draw without breaking the line: עברית العربية é (e + combining accent) é (one character)
- [ ] Wide characters: 日本語のテキスト 한국어 中文
- [ ] Zero-width joiner sequence stays one emoji (or falls back cleanly): 👩‍💻
- [ ] The editor stops taking text at about 1 MB (paste something huge to check); nothing crashes.

## 12. Open… (the read-only viewer)

**Open…** in the footer, then each file in `assets/notes-debug/`:

| File | Expect |
|---|---|
| `viewer-crlf-tabs.txt` | Opens in a tab beside Notes; line numbers; no wrapping (the long line scrolls sideways); tabs kept; CRLF shown as plain lines with no stray symbols |
| `viewer-utf8-bom.cfg` | Opens; the first line starts with `#`, with no `ï»¿` in front of it |
| `viewer-utf16le.txt` | Opens and reads normally (Windows "Unicode" text, like many Windows logs) |
| `viewer-utf16be.txt` | Opens and reads normally |
| `viewer-binary.bin` | Refused: "not a text file" |

- [ ] The tab shows the file name; the footer shows its lines and size; **Copy** copies the whole file; **Close** closes the tab and returns to Notes.
- [ ] Select text in the viewer with the mouse and Ctrl+C: copies just the selection.
- [ ] The viewed file is never saved into Notes, redacted or put in the Site Bundle.
- [ ] Too big: a file over 5 MB is refused with its size. Make one to test:
  - Windows (PowerShell): `fsutil file createnew big.log 6000000`
  - Linux: `head -c 6000000 /dev/zero | tr '\0' 'a' > big.log`

---

All boxes ticked: Notes is fine. Anything that doesn't match: that's the bug. 🐛
