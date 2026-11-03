//! Clients a few at a time: the +N and − chips on a switch, AP or firewall
//! card, and why a map has no clients to show.

use super::*;
use crate::map::{STEP, fewer_of, is_more, more_of};

/// Zoomed out past this, the chips would be too small to hit.
const MIN_CHIP_ZOOM: f32 = 0.28;

/// One chip on a card: its text, tooltip, and the clients its uplink shows
/// once clicked (None: as the level says).
pub(super) struct Chip {
    text: String,
    tip: String,
    to: Option<usize>,
}

/// The chips' font and height at zoom `z`.
fn chip_style(z: f32) -> (FontId, f32) {
    (
        FontId::proportional((10.5 * z).clamp(10.0, 12.5)),
        (17.0 * z).clamp(15.0, 20.0),
    )
}

/// About how wide a chip with `text` is drawn in `font`, at least `h`.
fn chip_width(text: &str, font: &FontId, h: f32) -> f32 {
    (text.chars().count() as f32 * font.size * 0.6 + 12.0).max(h)
}

impl MapView {
    /// Show `n` clients under the uplink `p` whatever the level; None: as
    /// the level says. The uplink stays where it is on screen, at this
    /// zoom, so its chips are still under the pointer for the next click.
    pub(super) fn reveal_step(&mut self, p: usize, n: Option<usize>) {
        let Some(id) = self.map.nodes.get(p).map(|n| n.id.clone()) else {
            return;
        };
        match n {
            Some(n) => self.reveal.insert(id.clone(), n),
            None => self.reveal.remove(&id),
        };
        self.hold = Some(id);
        self.follow = false;
        self.aim = None;
    }

    /// After the rebuild a step caused: move the view with the stepped
    /// uplink's new place, so it doesn't move on screen.
    pub(super) fn hold_still(&mut self) {
        let Some(id) = self.hold.take() else {
            return;
        };
        let (Some(i), Some(&now)) = (self.index(&id), self.pos.get(&id)) else {
            return;
        };
        let Some(to) = self.target(i) else {
            return;
        };
        let d = to - now;
        for p in self.pos.values_mut() {
            *p += d;
        }
        self.pan -= d * self.zoom;
    }

    /// What can be done with card `i`'s clients, zoomed in or not: "+N"
    /// while it has clients not drawn, and "−" once stepped open.
    pub(super) fn steps(&self, i: usize) -> Vec<Chip> {
        let n = &self.map.nodes[i];
        let mut out = Vec::new();
        // A saved map keeps the cards it was saved with.
        if self.frozen.is_some() {
            return out;
        }
        let Some(&(shown, total)) = self.map.under.get(&n.id) else {
            return out;
        };
        if total > shown {
            let next = more_of(shown, total);
            let tip = if next == usize::MAX {
                format!("Show the {} clients not drawn here", total - shown)
            } else {
                format!(
                    "Show {STEP} more of the {} clients not drawn here, best first \
                     (Shift+click: all of them)",
                    total - shown
                )
            };
            out.push(Chip {
                text: format!("+{}", total - shown),
                tip,
                to: Some(next),
            });
        }
        let stepped = self.reveal.get(&n.id).copied();
        if shown > 0 && stepped.is_some_and(|s| s > 0) {
            out.push(Chip {
                text: "−".into(),
                tip: "Hide the clients shown here again".into(),
                to: fewer_of(stepped, self.map.cap),
            });
        }
        out
    }

    /// The chips card `i` wears on the map. None when zoomed out too far
    /// for them.
    fn chips(&self, i: usize) -> Vec<Chip> {
        if self.zoom < MIN_CHIP_ZOOM || self.map.nodes[i].gone {
            return Vec::new();
        }
        self.steps(i)
    }

    /// Whether card `i` wears a +N chip on screen: it then stands in for
    /// the card's "N clients" count.
    pub(super) fn has_more_chip(&self, i: usize) -> bool {
        self.chips(i).iter().any(|c| c.text.starts_with('+'))
    }

    /// Each chip on screen in the canvas `rect`, with its card. They hang
    /// off the card's bottom right corner, a third over the card, so no
    /// line on the card has to make room for them.
    fn chip_rects(&self, rect: Rect) -> Vec<(usize, Rect, Chip)> {
        let z = self.zoom;
        let mut out = Vec::new();
        if z < MIN_CHIP_ZOOM || self.map.under.is_empty() {
            return out;
        }
        let picked = self.selection();
        let lit = if self.search.trim().is_empty() {
            HashSet::new()
        } else {
            self.lit(&picked).0
        };
        let (font, h) = chip_style(z);
        for (i, n) in self.map.nodes.iter().enumerate() {
            let Some(&(_, total)) = self.map.under.get(&n.id) else {
                continue;
            };
            let Some(&at) = self.pos.get(&n.id).filter(|_| !self.faded(i, &lit)) else {
                continue;
            };
            let card = Rect::from_min_size(self.to_screen(rect, at), layout::card_size(n) * z);
            if !card.intersects(rect) {
                continue;
            }
            // Clear of the warning dot on a card stacked below. The +N has
            // a slot of its own, as wide as it can get, so − stays put as
            // the count falls and once it's gone: a quick run of clicks on
            // +N never lands on −.
            let right = card.max.x - (8.0 * z).max(9.0);
            let slot = chip_width(&format!("+{total}"), &font, h);
            let top = card.max.y - h * 0.35;
            for c in self.chips(i) {
                let (r, w) = if c.text.starts_with('+') {
                    (right, slot)
                } else {
                    (right - slot - 4.0, chip_width(&c.text, &font, h))
                };
                let chip = Rect::from_min_max(pos2(r - w, top), pos2(r, top + h));
                out.push((i, chip, c));
            }
        }
        out
    }

    /// Whether the screen point `at` is on a chip: the chip's tooltip
    /// shows there, not the card's.
    pub(super) fn on_chip(&self, rect: Rect, at: Pos2) -> bool {
        self.chip_rects(rect).iter().any(|(_, r, _)| r.contains(at))
    }

    /// "+N" on each card with clients it doesn't draw (the next few, best
    /// first; Shift+click for all of them), and "−" on one stepped open to
    /// hide them again.
    pub(super) fn reveal_chips(&mut self, ui: &mut Ui, rect: Rect) {
        let chips = self.chip_rects(rect);
        if chips.is_empty() {
            return;
        }
        let vis = ui.visuals().clone();
        let p = ui.painter_at(rect);
        let (font, h) = chip_style(self.zoom);
        let mut asked = None;
        for (k, (i, chip, c)) in chips.into_iter().enumerate() {
            let n = &self.map.nodes[i];
            let accent = self.accent(n);
            let resp = ui
                .interact(chip, ui.id().with(("reveal", &n.id, k)), Sense::click())
                .on_hover_text(&c.tip);
            let fill = vis
                .window_fill
                .lerp_to_gamma(accent, if resp.hovered() { 0.42 } else { 0.2 });
            p.rect(
                chip,
                (h / 2.0) as u8,
                fill,
                Stroke::new(1.0, accent),
                StrokeKind::Inside,
            );
            p.text(
                chip.center(),
                egui::Align2::CENTER_CENTER,
                &c.text,
                font.clone(),
                vis.strong_text_color(),
            );
            if resp.clicked() {
                let all = ui.input(|i| i.modifiers.shift) && c.text.starts_with('+');
                asked = Some((i, if all { Some(usize::MAX) } else { c.to }));
            }
        }
        if let Some((i, to)) = asked {
            self.reveal_step(i, to);
        }
    }

    /// The uplink card `i`'s clients hang off (its own, or the one a +N
    /// more card follows on from), with (clients drawn, clients known).
    fn reveal_of(&self, i: usize) -> Option<(usize, usize, usize)> {
        let n = &self.map.nodes[i];
        let p = if is_more(n) { n.parent()? } else { i };
        let &(shown, total) = self.map.under.get(&self.map.nodes[p].id)?;
        Some((p, shown, total))
    }

    /// What can be done with card `i`'s clients (an uplink's, or the one a
    /// +N more card follows on from), as its +N and − chips do: one list
    /// for the card menu and the panel's buttons, so they always offer the
    /// same steps.
    pub(super) fn reveal_items(&self, i: usize) -> Vec<RevealItem> {
        let Some((p, shown, total)) = self.reveal_of(i) else {
            return Vec::new();
        };
        let s = |n: usize| if n == 1 { "" } else { "s" };
        let mut out = Vec::new();
        for c in self.steps(p) {
            if c.text.starts_with('+') {
                let next = c.to.unwrap_or(usize::MAX);
                if next != usize::MAX {
                    out.push(RevealItem {
                        label: format!("Show {} more client{}", next - shown, s(next - shown)),
                        short: format!("Show {} more", next - shown),
                        tip: "Cards for the next few, best first",
                        uplink: p,
                        to: Some(next),
                    });
                }
                // The menu has no "N of M shown" beside it: all of them is
                // the uplink's total, and once some show, the rest.
                let left = total - shown;
                out.push(RevealItem {
                    label: if shown == 0 {
                        format!("Show all {total} client{}", s(total))
                    } else {
                        format!("Show the other {left} client{}", s(left))
                    },
                    short: format!("Show all {left}"),
                    tip: "A card for each of them",
                    uplink: p,
                    to: Some(usize::MAX),
                });
            } else {
                let (label, tip) = match c.to {
                    None => ("Fold clients", "Back to what the Clients level shows"),
                    Some(_) => ("Hide clients", "Hide the clients shown here again"),
                };
                out.push(RevealItem {
                    label: label.into(),
                    short: label.into(),
                    tip,
                    uplink: p,
                    to: c.to,
                });
            }
        }
        out
    }

    /// Card `i`'s clients in the detail panel (an uplink's, or the one a +N
    /// more card follows on from): how many show, and buttons for the next
    /// few, all of them, or fewer. Near the top, so they stay put as the
    /// list below them changes. Returns the step asked for.
    pub(super) fn reveal_row(&self, ui: &mut Ui, i: usize) -> Option<(usize, Option<usize>)> {
        if self.frozen.is_some() {
            return None;
        }
        let (_, shown, total) = self.reveal_of(i)?;
        let mut asked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.weak(format!(
                "{shown} of {total} client{} shown",
                if total == 1 { "" } else { "s" }
            ));
            for it in self.reveal_items(i) {
                if ui.small_button(&it.short).on_hover_text(it.tip).clicked() {
                    asked = Some((it.uplink, it.to));
                }
            }
        });
        asked
    }
}

/// One step for a card's clients, as the card menu and the detail panel
/// offer it.
pub(super) struct RevealItem {
    /// The menu's label, which says what it's about ("Show 8 more clients").
    pub label: String,
    /// The panel's, beside its "N of M clients shown" ("Show 8 more").
    pub short: String,
    pub tip: &'static str,
    /// The uplink stepped.
    pub uplink: usize,
    /// The clients it shows then (None: as the level says).
    pub to: Option<usize>,
}

/// Why the Clients control has nothing to show, for its tooltip: what the
/// loaded files leave out, and what would add hosts.
pub(super) fn no_clients_why(src: &Sources) -> &'static str {
    let unifi_none = src
        .export
        .unifi()
        .is_some_and(|u| u.1.clients == crate::unifi::Clients::None);
    if unifi_none {
        "This map has no client data: the UniFi backup was read without its client table \
         (Config Export, UniFi clients: none). Pick anonymized or full there and open it \
         again, or run a Subnet Sweep to add hosts"
    } else if src.export.mapnetwork().is_some() {
        "This map has no client data: /mapnetwork drew infrastructure only. Run a Subnet \
         Sweep on the site's network (Scan) to add its hosts, or open the site's UniFi \
         backup with its clients"
    } else {
        "This map has no client data; run a Subnet Sweep to add hosts"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stepped_uplink_stays_put_on_screen() {
        let mut v = MapView {
            zoom: 0.5,
            pan: vec2(10.0, 20.0),
            ..Default::default()
        };
        for id in ["sw", "pc"] {
            v.map.nodes.push(Node {
                id: id.into(),
                ..Default::default()
            });
        }
        v.map.under.insert("sw".into(), (0, 40));
        v.pos.insert("sw".into(), pos2(0.0, 0.0));
        v.pos.insert("pc".into(), pos2(0.0, 100.0));
        v.follow = true;
        v.reveal_step(0, Some(STEP));
        assert!(!v.follow, "no fit to the grown map");
        // The rebuild laid it out 100 to the right.
        v.lay.pos = vec![Some(pos2(100.0, 0.0)), Some(pos2(0.0, 100.0))];
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let before = v.to_screen(rect, v.pos["sw"]);
        let pc = v.to_screen(rect, v.pos["pc"]);
        v.hold_still();
        assert_eq!(v.pos["sw"], pos2(100.0, 0.0));
        assert_eq!(v.to_screen(rect, v.pos["sw"]), before);
        // Everything else moves with it, then eases to its new place.
        assert_eq!(v.to_screen(rect, v.pos["pc"]), pc);
        assert!(v.hold.is_none());
    }

    #[test]
    fn chips_offer_the_next_few_then_hide_them_again() {
        let mut v = MapView {
            zoom: 1.0,
            ..Default::default()
        };
        v.map.nodes.push(Node {
            id: "sw".into(),
            zone: Zone::Switch,
            ..Default::default()
        });
        v.map.under.insert("sw".into(), (0, 40));
        let chips = |v: &MapView| -> Vec<(String, Option<usize>)> {
            v.chips(0).into_iter().map(|c| (c.text, c.to)).collect()
        };
        assert_eq!(chips(&v), [("+40".into(), Some(STEP))]);
        assert!(v.has_more_chip(0));
        // Stepped open: the next eight, and − back to the level (Off).
        v.reveal_step(0, Some(STEP));
        v.map.under.insert("sw".into(), (STEP, 40));
        assert_eq!(chips(&v), [("+32".into(), Some(16)), ("−".into(), None)]);
        // All shown: only −.
        v.map.under.insert("sw".into(), (40, 40));
        v.reveal_step(0, Some(usize::MAX));
        assert_eq!(chips(&v), [("−".into(), None)]);
        assert!(!v.has_more_chip(0));
        // Too far out to hit: none.
        v.zoom = 0.2;
        assert!(v.chips(0).is_empty() && !v.has_more_chip(0));
        assert_eq!(v.steps(0).len(), 1, "the panel still offers them");
        v.reveal_step(0, None);
        assert!(v.reveal.is_empty());
    }
}
