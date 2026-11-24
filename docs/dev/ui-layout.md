# UI layout spec

The detail behind CLAUDE.md's "UI: field laptop first". Read this before changing layout, panels, the nav, the run bar or table columns. Code comments that cite a breakpoint or a drop order point here.

Side-panel rework mocks: https://claude.ai/artifact/6JjJnabWUj1QjwaVfc65zh. Every step of that rework meets this spec.

## Window sizes

| Size (points) | What it is | Status |
|---|---|---|
| 1280 × 720 | 1080p laptop at 150% scaling | the target |
| 1120 × 720 | what the app opens at | must keep working |
| 620 × 400 | the smallest the window allows | must keep working |

Layout goes by **window width**, never screen size: half a 4K monitor gets the small-laptop layout.

## Breakpoints

| Window width | Nav | Inspector | Map side panel |
|---|---|---|---|
| **1440 and up** (`WIDE_FROM`, src/nav.rs) | open, 216 | docked, 360 | docked, 272 |
| **1100–1439** (`OPEN_FROM`) | open, 216 (on the Map too) | docked at 320 on tool pages; floats over the canvas on the Map | opening it closes the inspector |
| **under 1100** | folds to the icon rail, 56 | floats over the content, Esc closes it | floats over the content, Esc closes it |

Also under 1100: run-bar options wrap (src/tools/ping.rs), and Notes' Split falls back to Edit (src/tools/notes.rs).

## Height

Height is the tighter limit. At 720 tall, the title bar, status bar and run bar together stay **under 130 points** (src/chrome.rs), so results get most of the window.

## The Map

The Map keeps its zoom when a panel opens. Panels float over the canvas; they never shrink it.

## Table columns

Columns drop in a fixed order as the window narrows: **Vendor, then MAC, then TTL** (src/tools/sweep.rs). Anything a row loses still shows in the inspector.

## Click targets

At least 28 points tall (`TARGET_H`), for a trackpad on a tailgate.
