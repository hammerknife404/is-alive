# Notes format test 📝

Open this with **Open…** in the Notes tab to read it as plain text, or paste it into the editor and switch to **Split** or **Preview** to see every element drawn. Every line says what it should look like.

## Headings

# H1: largest, bold, with a rule under it 🔥
## H2: a size down, with a rule under it 📌
### H3: no rule ⭐
#### H4: no rule 💡
##### H5: body size, bold ✅
###### H6: body size, bold ❌
####### Seven hashes: not a heading, plain text
#No space after the hash: plain text

## Inline styles

Plain text with no markup at all.
**Bold text** in the middle of a sentence.
*Italic with asterisks* and _italic with underscores_.
***Bold and italic together*** and **bold with *italic* inside**.
`inline code` and a path: `C:\ProgramData\is-alive\notes.md`.
A [link to the vendor](https://www.example.com) is shown, not followed.
snake_case_names and file_name.txt stay literal: no italics inside words.
2 * 3 * 4 = 24 keeps its asterisks (spaces around them).
An unclosed **bold runs to the end of the line
An unclosed `backtick stays a backtick.
Backslash escapes aren't supported: \*stars\* keeps the backslashes and goes italic.

## Redaction markers

Wi-Fi key: [🔑 REDACTED: password] (drawn in the warning colour on a tint)
Older marker: [REDACTED: secret] (same look)
Not a marker: [REDACTED (no colon, plain text)

## Lists

- Bullet with a dash 🖥
* Bullet with an asterisk 💻
+ Bullet with a plus 📱
  - Indented two spaces: nested one step 🔌
    - Indented four spaces: nested two steps 🔋
- Bullet with **bold**, *italic*, `code` and a [link](https://example.com)

1. Numbered one 📅
2. Numbered two ⏰
10. Double digits keep their number
3.Missing space: plain text, not a list

## Task boxes

- [ ] Open task (tick it in Preview; the source line changes to [x]) 🔧
- [x] Done task, struck through and dimmed 👍
- [X] Done with a capital X 👎
* [ ] Task with an asterisk bullet 🐛
  - [ ] Nested task 🚧
- [] No space in the box: an ordinary bullet

## Quotes

> A quote: accent bar on the left, text beside it 📝
> Quote with **bold**, *italic* and `code` inside ✨
>No space after the marker still quotes

## Code block

```
interface X0
  ip address 10.20.0.1 255.255.255.0
  # Markdown inside a code block stays literal: **not bold**, - not a list
  ping 10.20.0.1 -n 4   ✅
```

```bash
# A language after the fence is ignored; still one code block
nmap -sn 10.20.0.0/24
```

## Rules

Three dashes:

---

Three asterisks:

***

Three underscores:

___

## Emoji: the picker's 32

✅ ❌ ⚠ ❗ ❓ ⭐ 👍 👎 🔥 💡 📌 📝 🔒 🔓 🔑 🔧
🖥 💻 📱 📠 📞 🌐 📶 🔌 🔋 📦 📅 ⏰ 🐛 🚧 ✨ 😀

## Emoji: beyond the picker

Faces: 😀 😃 😄 😁 😆 😅 😂 🙂 😉 😊 😐 😑 😕 😟 😢 😭 😡 🤔 🙄 😴
Hands: 👍 👎 👌 👋 👏 🙏 ✋ ✌ 👉 👈 👆 👇
Gear and places: 🖨 🗄 🗃 🖱 ⌨ 🧰 🔨 🛠 🔩 📡 🛰 🏢 🏥 🏫 🏠
Status: ✔ ✖ ➕ ➖ ⛔ 🚫 🟢 🟡 🔴 🟠 🔵 ⚪ ⚫
Time and weather: ⌛ ⏳ ☀ ☁ ⚡ ❄ 🌧
Arrows: ⬆ ⬇ ⬅ ➡ ↩ 🔄 🔁

## Emoji edge cases

Variation selector (text, then emoji style): ⚠ ⚠️ ✔ ✔️ ☎ ☎️
Skin tone modifier: 👍🏽 👋🏿
Joined sequence (ZWJ): 👩‍💻 🧑‍🔧 🏳️‍🌈
Flags (two regional letters): 🇺🇸 🇨🇦 🇬🇧
Keycaps: 1️⃣ 2️⃣ #️⃣
Newer than the bundled font (may show as a box): 🫠 🫡 🪲
Emoji in a heading, a list, a quote, code and bold are above; here in italics: *🔥 hot fix*

## Mixed, as a real note

### 2026-10-31 14:05 (UTC-05:00 Central Standard Time, synced to time.windows.com)

**This machine**
- Computer: TECH-LAPTOP-07 (HARBOR.LOCAL)
- LAN: 10.20.0.57/24
- MAC: 3C:52:82:00:00:57
- Gateway: 10.20.0.1
- WAN: 203.0.113.42

- [x] Front firewall reachable from the LAN ✅
- [ ] Back office AP offline: check PoE on core port 7 ⚠
- [ ] Imaging (PACS) backup last ran 9 days ago 🔥

> Client says the Wi-Fi drops "every afternoon" 📶

Admin login is in the vault, not here: [🔑 REDACTED: password]

---

End of test. If any line above doesn't look as it says, that's a bug. 🐛
