//! What a map's legend lists, as plain data: one entry for each kind of
//! card, badge, link and box that is on the map now, with how many. The
//! canvas (`view::legend`) and the SVG export draw the same entries, each
//! with the code it draws the map with, so a legend can't drift from the
//! picture it explains.

use super::layout::{self, Group, Line};
use super::{Link, Map, Zone};

/// The legend's groups, in the order they're listed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Roles,
    Cards,
    Links,
    Boxes,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Roles => "ROLES",
            Section::Cards => "CARDS",
            Section::Links => "LINKS",
            Section::Boxes => "BOXES",
        }
    }
}

/// The colour a swatch is drawn in, named the way the map names it; each
/// renderer turns it into its own colour exactly as it does on the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ink {
    /// A role's colour.
    Role(Zone),
    /// The red of a pinned card that's missing from the latest data.
    Gone,
    /// The weak text colour (assumed links, site boxes).
    Weak,
    /// The warning badge's amber.
    Warn,
}

/// What a swatch shows.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Swatch {
    /// A card in its role's colour.
    Card,
    /// A card standing for several (a collapsed site, +N more clients): a
    /// card with a chip.
    Folded,
    /// A card with a dashed outline: pinned, then missing from the data.
    Gone,
    /// The warning badge in a card's corner.
    Warn,
    /// The pin on a card kept in the selection.
    Pin,
    /// A chip on a card (the uplink's port or a note).
    Chip,
    /// A link drawn as this kind.
    Line(Link),
    /// Several links into a box folded into one line, with a count; drawn
    /// as its strongest link.
    Trunk(Link),
    /// A selected card's path: lit, with packets flowing.
    Lit,
    /// A site's or a role's box.
    Box,
}

/// One line of the legend.
#[derive(Clone, PartialEq, Debug)]
pub struct Entry {
    pub section: Section,
    pub swatch: Swatch,
    pub ink: Ink,
    pub label: String,
    /// How many there are on the map, where that says something.
    pub count: Option<usize>,
    /// A role's row: clicking it fades that role out or back in.
    pub role: Option<Zone>,
}

impl Entry {
    fn new(section: Section, swatch: Swatch, ink: Ink, label: &str, count: Option<usize>) -> Self {
        Entry {
            section,
            swatch,
            ink,
            label: label.to_string(),
            count,
            role: None,
        }
    }
}

/// What a card hanging off `n` is drawn in: its role's colour, or the gone red.
fn accent(n: &super::Node) -> Ink {
    if n.gone { Ink::Gone } else { Ink::Role(n.zone) }
}

/// The one seen most; on a tie, the one seen last (lines are painted in
/// order, so of two as common the later is the one on top). Linear: there
/// are only a handful of distinct values.
fn most<T: Copy + PartialEq>(inks: &[T]) -> Option<T> {
    // Each value with how often it's seen and where it was seen last.
    let mut tally: Vec<(T, usize, usize)> = Vec::new();
    for (k, &i) in inks.iter().enumerate() {
        match tally.iter_mut().find(|t| t.0 == i) {
            Some(t) => {
                t.1 += 1;
                t.2 = k;
            }
            None => tally.push((i, 1, k)),
        }
    }
    tally.into_iter().max_by_key(|t| (t.1, t.2)).map(|t| t.0)
}

/// A slot in the legend's columns: a section's heading or an entry (its
/// index), with its column and row.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Slot {
    pub head: Option<Section>,
    pub entry: Option<usize>,
    pub col: usize,
    pub row: usize,
}

/// Lay `entries` out in columns of at most `rows` rows, each section under
/// its heading, top to bottom then left to right. A section that fits in a
/// column but not in what's left of this one starts the next; a longer one
/// runs on, but a heading never ends a column.
pub fn slots(entries: &[Entry], rows: usize) -> Vec<Slot> {
    let rows = rows.max(2);
    let mut out = Vec::new();
    let (mut col, mut row) = (0, 0);
    for (k, e) in entries.iter().enumerate() {
        if k == 0 || entries[k - 1].section != e.section {
            let len = 1 + entries[k..]
                .iter()
                .take_while(|x| x.section == e.section)
                .count();
            if row > 0 && (row + len > rows && len <= rows || row + 2 > rows) {
                col += 1;
                row = 0;
            }
            out.push(Slot {
                head: Some(e.section),
                entry: None,
                col,
                row,
            });
            row += 1;
        }
        if row >= rows {
            col += 1;
            row = 0;
        }
        out.push(Slot {
            head: None,
            entry: Some(k),
            col,
            row,
        });
        row += 1;
    }
    out
}

/// The link a line is drawn as: its strongest (one solid uplink makes a
/// trunk solid), as the canvas and the SVG draw it.
pub fn drawn_link(map: &Map, line: &Line) -> Link {
    line.kids
        .iter()
        .map(|&k| &map.nodes[k])
        .max_by_key(|m| m.link().map_or(0, layout::strength))
        .and_then(|n| n.link())
        .unwrap_or(Link::Wired)
}

/// The legend for `map` as laid out (`boxes`, and the `lines` it's drawn
/// with). `sites`: in site boxes (Sites), where a card's site chip isn't
/// drawn. `pinned` is how many cards are pinned; `live` is the canvas,
/// which also shows pins and the lit path (an export has neither).
pub fn entries(
    map: &Map,
    boxes: &[Group],
    lines: &[Line],
    sites: bool,
    pinned: usize,
    live: bool,
) -> Vec<Entry> {
    let mut out = Vec::new();
    let nodes = &map.nodes;
    let count = |f: &dyn Fn(&super::Node) -> bool| nodes.iter().filter(|n| f(n)).count();

    // Roles, as the sidebar lists them: the internet isn't one to fade.
    for z in Zone::ALL {
        let n = count(&|c| c.zone == z);
        if n > 0 && z != Zone::Internet {
            let mut e = Entry::new(
                Section::Roles,
                Swatch::Card,
                Ink::Role(z),
                z.name(),
                Some(n),
            );
            e.role = Some(z);
            out.push(e);
        }
    }

    // Cards drawn apart from their role.
    let internet = count(&|c| c.zone == Zone::Internet);
    if internet > 0 {
        out.push(Entry::new(
            Section::Cards,
            Swatch::Card,
            Ink::Role(Zone::Internet),
            "Internet / WAN",
            Some(internet),
        ));
    }
    let folded = |is: &dyn Fn(&super::Node) -> bool| -> Vec<Ink> {
        nodes.iter().filter(|c| is(c)).map(accent).collect()
    };
    let site = |c: &super::Node| c.id.starts_with("site:");
    // A +N more card, or a /mapnetwork map's own "+7 more" listing nobody.
    let more = |c: &super::Node| super::is_more(c);
    for (is, label) in [
        (&site as &dyn Fn(&super::Node) -> bool, "Collapsed site"),
        (&more, "More clients, folded"),
    ] {
        let inks = folded(is);
        if let Some(ink) = most(&inks) {
            out.push(Entry::new(
                Section::Cards,
                Swatch::Folded,
                ink,
                label,
                Some(inks.len()),
            ));
        }
    }
    let gone = count(&|c| c.gone);
    if gone > 0 {
        out.push(Entry::new(
            Section::Cards,
            Swatch::Gone,
            Ink::Gone,
            "Not in latest data",
            Some(gone),
        ));
    }
    if live && pinned > 0 {
        out.push(Entry::new(
            Section::Cards,
            Swatch::Pin,
            Ink::Weak,
            "Pinned",
            Some(pinned),
        ));
    }
    let flagged = count(&|c| !c.flags.is_empty());
    if flagged > 0 {
        out.push(Entry::new(
            Section::Cards,
            Swatch::Warn,
            Ink::Warn,
            "Has warnings",
            Some(flagged),
        ));
    }
    // A chip: the uplink's port or note, or one of the card's own (in
    // Sites the box names the site, so that chip isn't drawn).
    let chipped: Vec<Ink> = nodes
        .iter()
        .filter(|c| {
            c.up.as_ref().is_some_and(|u| u.2.is_some())
                || c.chips
                    .iter()
                    .any(|s| !(sites && c.site.as_deref() == Some(s.as_str())))
        })
        .map(accent)
        .collect();
    if let Some(ink) = most(&chipped) {
        out.push(Entry::new(
            Section::Cards,
            Swatch::Chip,
            ink,
            "Port, note or tag",
            None,
        ));
    }

    // Links: each kind drawn, in the colour most of its lines have, with
    // how many cards hang off their uplink that way (a trunk is drawn as
    // its strongest link, so a kind only ever in trunks isn't listed).
    // Which kinds and colours: from the lines the map shows, not the quiet
    // ones inside a Zones box (drawn only while lit); the counts: every
    // card's uplink.
    let links: Vec<Link> = lines
        .iter()
        .flat_map(|l| l.kids.iter().filter_map(|&k| nodes[k].link()))
        .collect();
    let mut kinds: Vec<(Link, Vec<Ink>)> = Vec::new();
    let mut trunks: Vec<(Link, Ink)> = Vec::new();
    for line in lines.iter().filter(|l| !l.quiet) {
        let link = drawn_link(map, line);
        let first = line
            .kids
            .iter()
            .map(|&k| &nodes[k])
            .max_by_key(|m| m.link().map_or(0, layout::strength));
        let Some(first) = first else { continue };
        let into = line.into.and_then(|g| boxes.get(g));
        let ink = match (into.and_then(|g| g.zone), link) {
            (Some(z), _) => Ink::Role(z),
            (None, Link::Backbone) => Ink::Role(Zone::Gateway),
            (None, Link::Assumed) => Ink::Weak,
            (None, _) => accent(first),
        };
        match kinds.iter_mut().find(|k| k.0 == link) {
            Some(k) => k.1.push(ink),
            None => kinds.push((link, vec![ink])),
        }
        if into.is_some() && line.kids.len() > 1 {
            trunks.push((link, ink));
        }
    }
    kinds.sort_by_key(|k| std::cmp::Reverse(layout::strength(k.0)));
    for (link, inks) in &kinds {
        let label = match link {
            Link::Backbone => "Internet uplink",
            Link::Wired => "Wired",
            Link::Wireless => "Wireless",
            Link::Assumed => "Assumed, not in the data",
        };
        // The internet's uplinks are the gateway's colour and assumed links
        // grey wherever they don't run into a role's box: that, if any
        // shown are.
        let own = match link {
            Link::Backbone => Some(Ink::Role(Zone::Gateway)),
            Link::Assumed => Some(Ink::Weak),
            _ => None,
        };
        let ink = own
            .filter(|i| inks.contains(i))
            .or_else(|| most(inks))
            .unwrap_or(Ink::Weak);
        let n = links.iter().filter(|l| *l == link).count();
        out.push(Entry::new(
            Section::Links,
            Swatch::Line(*link),
            ink,
            label,
            Some(n),
        ));
    }
    if let Some((link, ink)) = most(&trunks) {
        out.push(Entry::new(
            Section::Links,
            Swatch::Trunk(link),
            ink,
            "Links into a box, ×count",
            Some(trunks.len()),
        ));
    }
    if live && !lines.is_empty() {
        // In the colour of the links most often lit: the wired ones.
        let ink = kinds
            .iter()
            .find(|k| k.0 == Link::Wired)
            .or(kinds.first())
            .and_then(|k| most(&k.1))
            .unwrap_or(Ink::Role(Zone::Gateway));
        out.push(Entry::new(
            Section::Links,
            Swatch::Lit,
            ink,
            "Selected: path lit",
            None,
        ));
    }

    // Boxes.
    let site_boxes = boxes.iter().filter(|b| b.tree).count();
    if site_boxes > 0 {
        out.push(Entry::new(
            Section::Boxes,
            Swatch::Box,
            Ink::Weak,
            // Only the canvas has a title to click.
            if live {
                "Site (click title to fold)"
            } else {
                "Site"
            },
            Some(site_boxes),
        ));
    }
    let role_boxes: Vec<Ink> = boxes
        .iter()
        .filter(|b| !b.tree)
        .map(|b| b.zone.map_or(Ink::Weak, Ink::Role))
        .collect();
    if let Some(ink) = role_boxes.first() {
        out.push(Entry::new(
            Section::Boxes,
            Swatch::Box,
            *ink,
            "Role box",
            Some(role_boxes.len()),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::import::build;
    use crate::map::layout::{Mode, layout_in};
    use crate::map::{Input, Node};
    use std::collections::BTreeSet;

    fn example(i: usize, clients: bool) -> Map {
        let f = crate::skills::EXAMPLES[i].1;
        let doc = crate::mapnetwork::parse(crate::skills::file(f).unwrap()).unwrap();
        build(
            &doc,
            &Input {
                clients: if clients {
                    crate::map::Clients::Some
                } else {
                    crate::map::Clients::Off
                },
                ..Default::default()
            },
        )
    }

    /// The entries for a map laid out in `mode`, with what they should
    /// list worked out from the map and its lines directly.
    fn check(m: &Map, mode: Mode) -> Vec<Entry> {
        let lay = layout_in(m, &|_| true, mode);
        let lines = layout::lines(m, &lay, &|i| lay.pos[i], &|_| false);
        let e = entries(m, &lay.boxes, &lines, mode == Mode::Sites, 0, false);
        // Exactly the roles on the map, with their counts.
        let roles: BTreeSet<(Zone, usize)> = e
            .iter()
            .filter_map(|x| x.role.map(|z| (z, x.count.unwrap())))
            .collect();
        let want: BTreeSet<(Zone, usize)> = Zone::ALL
            .iter()
            .filter(|&&z| z != Zone::Internet)
            .map(|&z| (z, m.nodes.iter().filter(|n| n.zone == z).count()))
            .filter(|c| c.1 > 0)
            .collect();
        assert_eq!(roles, want, "{mode:?}");
        // Exactly the link kinds drawn.
        let kinds: Vec<u8> = e
            .iter()
            .filter_map(|x| match x.swatch {
                Swatch::Line(l) => Some(layout::strength(l)),
                _ => None,
            })
            .collect();
        // Quiet lines (inside a Zones box) aren't drawn unless lit.
        let shown = || lines.iter().filter(|l| !l.quiet);
        let drawn: BTreeSet<u8> = shown()
            .map(|l| layout::strength(drawn_link(m, l)))
            .collect();
        assert_eq!(kinds.iter().copied().collect::<BTreeSet<_>>(), drawn);
        assert_eq!(kinds.len(), drawn.len(), "one entry per kind");
        // Each kind's swatch in a colour one of its shown lines is drawn
        // in: a box's for a line into one, else as the map colours it.
        let ink = |l: &Line| {
            let n = l
                .kids
                .iter()
                .map(|&k| &m.nodes[k])
                .max_by_key(|n| n.link().map_or(0, layout::strength))
                .unwrap();
            match (l.into.and_then(|g| lay.boxes[g].zone), drawn_link(m, l)) {
                (Some(z), _) => Ink::Role(z),
                (None, Link::Backbone) => Ink::Role(Zone::Gateway),
                (None, Link::Assumed) => Ink::Weak,
                (None, _) => accent(n),
            }
        };
        for x in &e {
            if let Swatch::Line(k) | Swatch::Trunk(k) = x.swatch {
                assert!(
                    shown().any(|l| drawn_link(m, l) == k && ink(l) == x.ink),
                    "{mode:?}: {} drawn in {:?}, which no line of it is",
                    x.label,
                    x.ink
                );
            }
        }
        // Trunks only where links fold into a box.
        let trunks = lines
            .iter()
            .filter(|l| l.into.is_some() && l.kids.len() > 1)
            .count();
        let trunk = e.iter().find(|x| matches!(x.swatch, Swatch::Trunk(_)));
        assert_eq!(trunk.and_then(|t| t.count), (trunks > 0).then_some(trunks));
        // Not live: no pins and no lit path in an export.
        assert!(
            !e.iter()
                .any(|x| matches!(x.swatch, Swatch::Pin | Swatch::Lit))
        );
        e
    }

    fn has(e: &[Entry], s: Swatch) -> bool {
        e.iter().any(|x| x.swatch == s)
    }

    /// The labels of a section's entries, in order.
    fn labels(e: &[Entry], s: Section) -> Vec<&str> {
        e.iter()
            .filter(|x| x.section == s)
            .map(|x| x.label.as_str())
            .collect()
    }

    #[test]
    fn acme_lists_what_is_on_it() {
        let m = example(0, false);
        for mode in Mode::ALL {
            let e = check(&m, mode);
            assert_eq!(
                labels(&e, Section::Roles),
                [
                    "Gateway / firewall",
                    "Switches",
                    "Access points",
                    "Network gear (inferred)",
                    "Servers / NAS",
                    "Printers / cameras"
                ]
            );
            // HA firewalls on two WANs: four internet uplinks, the rest wired.
            let links: Vec<(Swatch, Option<usize>)> = e
                .iter()
                .filter(|x| matches!(x.swatch, Swatch::Line(_)))
                .map(|x| (x.swatch, x.count))
                .collect();
            assert_eq!(
                links,
                [
                    (Swatch::Line(Link::Backbone), Some(4)),
                    (Swatch::Line(Link::Wired), Some(32))
                ],
                "{mode:?}"
            );
            assert!(has(&e, Swatch::Line(Link::Backbone)), "{mode:?}");
            assert!(has(&e, Swatch::Line(Link::Wired)), "{mode:?}");
            assert!(e.iter().any(|x| x.label == "Internet / WAN"));
            // Zones has a box per role, Sites one for its one site.
            let boxed = matches!(mode, Mode::Zones | Mode::Sites);
            assert_eq!(has(&e, Swatch::Box), boxed, "{mode:?}");
            // Links into a box fold into one counted line.
            let trunk = e.iter().any(|x| matches!(x.swatch, Swatch::Trunk(_)));
            assert_eq!(trunk, boxed, "{mode:?}");
            assert!(!has(&e, Swatch::Gone) && !has(&e, Swatch::Folded));
        }
        // Clients on: their role, and the +N more cards if any fold.
        let m = example(0, true);
        let e = check(&m, Mode::Cluster);
        assert!(e.iter().any(|x| x.role == Some(Zone::Client)));
        assert_eq!(
            has(&e, Swatch::Folded),
            m.nodes.iter().any(|n| n.id.starts_with("more:"))
        );
    }

    #[test]
    fn contoso_lists_its_sites_and_collapsed_ones() {
        let mut m = example(1, false);
        for mode in Mode::ALL {
            let e = check(&m, mode);
            assert_eq!(
                labels(&e, Section::Roles),
                [
                    "Gateway / firewall",
                    "Switches",
                    "Access points",
                    "Unidentified"
                ]
            );
            // Clinics' own lines to the internet, and switches the data
            // doesn't place: wired and assumed links, nothing wireless.
            let links: Vec<(Swatch, Option<usize>)> = e
                .iter()
                .filter(|x| matches!(x.swatch, Swatch::Line(_)))
                .map(|x| (x.swatch, x.count))
                .collect();
            assert_eq!(
                links,
                [
                    (Swatch::Line(Link::Backbone), Some(2)),
                    (Swatch::Line(Link::Wired), Some(55)),
                    (Swatch::Line(Link::Assumed), Some(24))
                ],
                "{mode:?}"
            );
        }
        let e = check(&m, Mode::Sites);
        let sites = m.sites().len();
        let b = e.iter().find(|x| x.swatch == Swatch::Box).unwrap();
        assert_eq!((b.ink, b.count), (Ink::Weak, Some(sites)));
        // A picture has no title to click; the canvas says it does.
        assert_eq!(b.label, "Site");
        let lay = layout_in(&m, &|_| true, Mode::Sites);
        let lines = layout::lines(&m, &lay, &|i| lay.pos[i], &|_| false);
        let live = entries(&m, &lay.boxes, &lines, true, 0, true);
        let b = live.iter().find(|x| x.swatch == Swatch::Box).unwrap();
        assert_eq!(b.label, "Site (click title to fold)");
        assert!(!has(&e, Swatch::Folded));
        // Collapse two sites: a Collapsed site entry counting them.
        let close: std::collections::HashSet<String> = m.sites().into_iter().take(2).collect();
        m.close_sites(&close);
        let e = check(&m, Mode::Sites);
        let f = e.iter().find(|x| x.swatch == Swatch::Folded).unwrap();
        assert_eq!((f.label.as_str(), f.count), ("Collapsed site", Some(2)));
        for mode in [Mode::Cluster, Mode::Tree, Mode::Zones] {
            check(&m, mode);
        }
    }

    #[test]
    fn most_is_the_commonest_and_the_one_on_top_on_a_tie() {
        assert_eq!(most::<u8>(&[]), None);
        assert_eq!(most(&[1, 2, 2]), Some(2));
        assert_eq!(most(&[2, 2, 1]), Some(2));
        // A tie: the one seen (drawn) last.
        assert_eq!(most(&[1, 2]), Some(2));
        assert_eq!(most(&[1, 2, 2, 1]), Some(1));
        assert_eq!(most(&[3, 1, 2, 2, 1]), Some(1));
    }

    #[test]
    fn sections_keep_together_in_columns() {
        let e = |s: Section| Entry::new(s, Swatch::Card, Ink::Weak, "x", None);
        let list: Vec<Entry> = [Section::Roles; 4]
            .into_iter()
            .chain([Section::Cards; 2])
            .chain([Section::Links; 3])
            .map(e)
            .collect();
        let at = |v: &[Slot]| -> Vec<(usize, usize)> { v.iter().map(|s| (s.col, s.row)).collect() };
        // Roles fill the first column; cards and links, too long to share
        // the next, take one each.
        let v = slots(&list, 6);
        assert_eq!(v.len(), list.len() + 3);
        assert_eq!(
            at(&v),
            [
                (0, 0),
                (0, 1),
                (0, 2),
                (0, 3),
                (0, 4),
                (1, 0),
                (1, 1),
                (1, 2),
                (2, 0),
                (2, 1),
                (2, 2),
                (2, 3)
            ]
        );
        // A section longer than a column runs on, its heading kept with it.
        let v = slots(&list, 3);
        assert!(v.iter().all(|s| s.row < 3));
        for w in v.windows(2) {
            if w[0].head.is_some() {
                assert_eq!(w[0].col, w[1].col, "a heading never ends a column");
            }
        }
        // One column: everything in order.
        let v = slots(&list, 100);
        assert!(v.iter().enumerate().all(|(i, s)| s.col == 0 && s.row == i));
    }

    #[test]
    fn live_adds_pins_and_the_lit_path_and_badges_follow_the_cards() {
        let mut m = Map {
            nodes: (0..3)
                .map(|i| Node {
                    id: format!("n{i}"),
                    zone: [Zone::Internet, Zone::Gateway, Zone::Server][i],
                    up: (i > 0).then(|| {
                        let link = [Link::Backbone, Link::Backbone, Link::Assumed][i];
                        (i - 1, link, None)
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        m.nodes[2].flags.push("telnet".into());
        m.nodes[2].gone = true;
        let lay = layout_in(&m, &|_| true, Mode::Cluster);
        let lines = layout::lines(&m, &lay, &|i| lay.pos[i], &|_| false);
        let e = entries(&m, &lay.boxes, &lines, false, 1, true);
        assert!(has(&e, Swatch::Pin) && has(&e, Swatch::Lit));
        assert!(has(&e, Swatch::Warn) && has(&e, Swatch::Gone));
        let a = e
            .iter()
            .find(|x| x.swatch == Swatch::Line(Link::Assumed))
            .unwrap();
        assert_eq!((a.ink, a.count), (Ink::Weak, Some(1)));
        assert!(!has(&e, Swatch::Line(Link::Wireless)));
        assert!(!has(&e, Swatch::Chip));
    }
}
