//! Clients a few at a time: the +N and − chips on a switch, AP or firewall
//! card, and why a map has no clients to show.

use super::*;
use crate::map::{STEP, fewer_of, more_of};

/// Zoomed out past this, the chips would be too small to hit.
const MIN_CHIP_ZOOM: f32 = 0.28;

/// One chip on a card: its text, tooltip, and the clients its uplink shows
/// once clicked (None: as the level says).
struct Chip {
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
    /// the level says.
    pub(super) fn step(&mut self, p: usize, n: Option<usize>) {
        let Some(id) = self.map.nodes.get(p).map(|n| n.id.clone()) else {
            return;
        };
        match n {
            Some(n) => self.reveal.insert(id, n),
            None => self.reveal.remove(&id),
        };
    }

    /// The chips card `i` wears: "+N" while it has clients not drawn, and
    /// "−" once stepped open. None when zoomed out too far for them.
    fn chips(&self, i: usize) -> Vec<Chip> {
        let n = &self.map.nodes[i];
        let mut out = Vec::new();
        let Some(&(shown, total)) = self.map.under.get(&n.id) else {
            return out;
        };
        if self.zoom < MIN_CHIP_ZOOM || n.gone {
            return out;
        }
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

    /// Whether card `i` wears a +N chip on screen: it then stands in for
    /// the card's "N clients" count.
    pub(super) fn has_more_chip(&self, i: usize) -> bool {
        self.chips(i).iter().any(|c| c.text.starts_with('+'))
    }

    /// Screen width the chips take from the bottom right of card `i`, so
    /// its address line stops short of them.
    pub(super) fn chip_room(&self, i: usize) -> f32 {
        let (font, h) = chip_style(self.zoom);
        self.chips(i)
            .iter()
            .map(|c| chip_width(&c.text, &font, h) + 4.0)
            .sum::<f32>()
    }

    /// "+N" on each card with clients it doesn't draw (the next few, best
    /// first; Shift+click for all of them), and "−" on one stepped open to
    /// hide them again. They sit in the card's bottom right corner.
    pub(super) fn reveal_chips(&mut self, ui: &mut Ui, rect: Rect) {
        let z = self.zoom;
        if z < MIN_CHIP_ZOOM || self.map.under.is_empty() {
            return;
        }
        let picked = self.selection();
        let lit = if self.search.trim().is_empty() {
            HashSet::new()
        } else {
            self.lit(&picked).0
        };
        let vis = ui.visuals().clone();
        let p = ui.painter_at(rect);
        let (font, h) = chip_style(z);
        let mut asked = None;
        for (i, n) in self.map.nodes.iter().enumerate() {
            if !self.map.under.contains_key(&n.id) || self.faded(i, &lit) {
                continue;
            }
            let Some(&at) = self.pos.get(&n.id) else {
                continue;
            };
            let card = Rect::from_min_size(self.to_screen(rect, at), layout::card_size(n) * z);
            if !card.intersects(rect) {
                continue;
            }
            let accent = self.accent(n);
            let mut right = card.max.x - (8.0 * z).max(4.0);
            let bottom = card.max.y - (5.0 * z).max(3.0);
            for (k, c) in self.chips(i).into_iter().enumerate() {
                let w = chip_width(&c.text, &font, h);
                let chip = Rect::from_min_max(pos2(right - w, bottom - h), pos2(right, bottom));
                right -= w + 4.0;
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
        }
        if let Some((i, to)) = asked {
            self.step(i, to);
        }
    }
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
        assert!(v.has_more_chip(0) && v.chip_room(0) > 0.0);
        // Stepped open: the next eight, and − back to the level (Off).
        v.step(0, Some(STEP));
        v.map.under.insert("sw".into(), (STEP, 40));
        assert_eq!(chips(&v), [("+32".into(), Some(16)), ("−".into(), None)]);
        // All shown: only −.
        v.map.under.insert("sw".into(), (40, 40));
        v.step(0, Some(usize::MAX));
        assert_eq!(chips(&v), [("−".into(), None)]);
        assert!(!v.has_more_chip(0));
        // Too far out to hit: none.
        v.zoom = 0.2;
        assert!(v.chips(0).is_empty() && v.chip_room(0) == 0.0);
        v.step(0, None);
        assert!(v.reveal.is_empty());
    }
}
