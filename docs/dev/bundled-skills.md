# Bundled skills: reference

The detail behind CLAUDE.md's "Bundled skills ship with the app". Read this before changing anything a Site Bundle carries, or anything in `skills/`.

`skills/unifidump` and `skills/mapnetwork` ship inside every Site Bundle, and the Map opens their example maps. They're how Claude reads what is-alive writes, so they're part of the change, not docs to catch up on later.

## What counts as a change to the bundle

Any of these updates `skills/` in the same PR:

- a file added to, removed from or renamed in the zip
- a manifest key
- an `inventory.json` field, or a change in what one holds (e.g. a MAC that can now come from somewhere new)
- what `results.json` spans
- what is-alive reads back from a `/mapnetwork` map

The same PR also updates the matching `*_WHAT` text in `src/tools/bundle.rs` (the manifest's description of each file) and the inventory's `notes` in `src/tools/sweep.rs`.

## Tests that hold the line

| Test | Where | Fails when |
|---|---|---|
| `the_bundled_mapnetwork_skill_knows_every_file` | src/bundle.rs | a bundle file or manifest key isn't named in `skills/mapnetwork/SKILL.md` |
| inventory field check | src/tools/sweep.rs (reads `skills/mapnetwork/SKILL.md`) | a host field in `inventory.json` isn't explained there |
| `the_bundled_skills_carry_no_full_macs` | src/skills.rs | a full MAC appears anywhere in `skills/` |
| `skills/mapnetwork/scripts/test_engine_contract.py` | CI | the map engine's contract breaks |

Fix a failure by explaining the new thing in the skill. Don't loosen the test.

## Syncing from the owner's source

`skills/` is a scrubbed copy that `scripts/sync-skills.py` makes from the owner's own skills. The owner patches their source themselves; don't hand them patches unless they ask. If a sync brings older text back into `skills/`, the tests above fail: restore the bundled text from git and point it out.

## Scrub

Text in `skills/` is public. Fictional names and documentation addresses only: no real site, client or person names.

No full MAC in any form. Both the test and `scripts/sync-skills.py` catch:

- colon or dash pairs (`aa:bb:cc:dd:ee:ff`, `aa-bb-…`)
- dotted quads (`aabb.ccdd.eeff`)
- a serial that is a MAC
- a bare MAC inside an id
