//! Undo and redo for what the user changes by hand on a map: cards
//! dragged or put back, Re-layout, the layout, Clients, pins, sites
//! collapsed, roles faded out, clients shown or hidden on a card (+N, −,
//! Show all). Selecting cards
//! isn't a step, as in most editors.
//!
//! Each step keeps the whole of that state as it was (a small struct), so
//! undo is putting it back. The map compares it with the last one kept
//! once a frame: any difference is a step, named from what changed, and a
//! drag is one step however many frames it takes.

use super::*;
use std::collections::VecDeque;

/// Most steps kept; the oldest go first.
pub const STEPS: usize = 100;

/// What the user changed by hand on one map.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Edits {
    /// Cards dragged somewhere on the canvas.
    pub placed: HashMap<String, Pos2>,
    /// The layout picked (None: the map's default).
    pub mode: Option<Mode>,
    pub clients: Clients,
    /// Uplinks stepped on their own (+N, −, Show all): how many clients
    /// each shows, whatever the level.
    pub reveal: HashMap<String, usize>,
    /// Sites collapsed to one card.
    pub closed: HashSet<String>,
    /// Roles faded out.
    pub hidden: HashSet<Zone>,
    /// Pinned cards.
    pub kept: Vec<String>,
}

/// Steps to undo and redo, each with what it did ("move 3 cards"), and the
/// state as it is now, to tell the next change by.
pub struct History<T> {
    undo: VecDeque<(T, String)>,
    redo: Vec<(T, String)>,
    now: T,
}

impl<T: Default> Default for History<T> {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            now: T::default(),
        }
    }
}

impl<T> History<T> {
    /// The state as last kept.
    pub fn now(&self) -> &T {
        &self.now
    }

    /// The state changed by itself (a rebuild): not a step.
    pub fn settle(&mut self, now: T) {
        self.now = now;
    }

    /// The user changed the state to `now` by `what`: a step to undo, and
    /// nothing left to redo.
    pub fn record(&mut self, now: T, what: String) {
        let before = std::mem::replace(&mut self.now, now);
        self.undo.push_back((before, what));
        while self.undo.len() > STEPS {
            self.undo.pop_front();
        }
        self.redo.clear();
    }

    /// Step back: the state to put back, if there's a step.
    pub fn undo(&mut self) -> Option<&T> {
        let (before, what) = self.undo.pop_back()?;
        let after = std::mem::replace(&mut self.now, before);
        self.redo.push((after, what));
        Some(&self.now)
    }

    /// Step forward again after an undo.
    pub fn redo(&mut self) -> Option<&T> {
        let (after, what) = self.redo.pop()?;
        let before = std::mem::replace(&mut self.now, after);
        self.undo.push_back((before, what));
        Some(&self.now)
    }

    /// What Undo would undo.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.back().map(|s| s.1.as_str())
    }

    /// What Redo would do again.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.1.as_str())
    }

    /// Forget every step (another map).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

/// What changed from `a` to `b`, as Undo's tooltip names it ("move 3
/// cards"). `name` gives a card's label from its id.
pub fn describe(a: &Edits, b: &Edits, name: &dyn Fn(&str) -> String) -> String {
    // One card by name, more by count.
    let cards = |ids: &[&String]| match ids {
        [one] => name(one),
        more => format!("{} cards", more.len()),
    };
    let sites = |v: &[&String]| match v {
        [one] => (*one).clone(),
        more => format!("{} sites", more.len()),
    };
    if a.mode != b.mode {
        let m = b.mode.map_or("the default", Mode::name);
        return format!("switch to the {m} layout");
    }
    if a.clients != b.clients {
        return format!("Clients: {}", b.clients.name());
    }
    if a.closed != b.closed {
        let shut: Vec<&String> = sorted(b.closed.difference(&a.closed));
        let opened: Vec<&String> = sorted(a.closed.difference(&b.closed));
        return match (shut.is_empty(), opened.is_empty()) {
            (false, true) => format!("collapse {}", sites(&shut)),
            (true, false) => format!("expand {}", sites(&opened)),
            _ => "collapse and expand sites".into(),
        };
    }
    if a.hidden != b.hidden {
        let faded: Vec<&Zone> = b.hidden.difference(&a.hidden).collect();
        return match faded.as_slice() {
            [z] => format!("fade out {}", z.name()),
            [] => match a
                .hidden
                .difference(&b.hidden)
                .collect::<Vec<_>>()
                .as_slice()
            {
                [z] => format!("fade in {}", z.name()),
                _ => "fade roles in".into(),
            },
            _ => "fade roles out".into(),
        };
    }
    if a.reveal != b.reveal {
        // How many clients a card shows by itself (0: as the level says).
        let shown = |e: &Edits, k: &String| e.reveal.get(k).copied().unwrap_or(0);
        let mut changed: Vec<&String> = sorted(
            a.reveal
                .keys()
                .chain(b.reveal.keys())
                .filter(|k| shown(a, k) != shown(b, k)),
        );
        changed.dedup();
        let more = changed.iter().all(|k| shown(b, k) > shown(a, k));
        let all = changed.iter().all(|k| shown(b, k) == usize::MAX);
        return match (changed.as_slice(), more) {
            ([one], true) if all => format!("show all clients under {}", name(one)),
            ([one], true) => format!("show clients under {}", name(one)),
            ([one], false) => format!("hide clients under {}", name(one)),
            (_, true) => "show clients".into(),
            _ => "hide clients".into(),
        };
    }
    if a.kept != b.kept {
        let pinned: Vec<&String> = b.kept.iter().filter(|k| !a.kept.contains(k)).collect();
        let unpinned: Vec<&String> = a.kept.iter().filter(|k| !b.kept.contains(k)).collect();
        return match (pinned.is_empty(), unpinned.is_empty()) {
            (false, true) => format!("pin {}", cards(&pinned)),
            (true, false) => format!("unpin {}", cards(&unpinned)),
            _ => "pins".into(),
        };
    }
    if a.placed != b.placed {
        let moved: Vec<&String> = sorted(
            b.placed
                .iter()
                .filter(|(k, p)| a.placed.get(*k) != Some(p))
                .map(|(k, _)| k),
        );
        if moved.is_empty() {
            let back: Vec<&String> = sorted(a.placed.keys().filter(|k| !b.placed.contains_key(*k)));
            return format!("put back {}", cards(&back));
        }
        return format!("move {}", cards(&moved));
    }
    "change".into()
}

/// The ids in order, so a name picked from several is always the same.
fn sorted<'a, T: Ord + 'a>(it: impl Iterator<Item = &'a T>) -> Vec<&'a T> {
    let mut v: Vec<&T> = it.collect();
    v.sort();
    v
}

impl MapView {
    /// What the user changed by hand, as it is now.
    pub(super) fn edits(&self) -> Edits {
        Edits {
            placed: self.placed.clone(),
            mode: self.mode,
            clients: self.clients,
            reveal: self.reveal.clone(),
            closed: self.closed.clone(),
            hidden: self.hidden.clone(),
            kept: self.kept.clone(),
        }
    }

    /// The edits differ from `e`, checked without copying them (every
    /// frame).
    pub(super) fn edits_differ(&self, e: &Edits) -> bool {
        self.placed != e.placed
            || self.mode != e.mode
            || self.clients != e.clients
            || self.reveal != e.reveal
            || self.closed != e.closed
            || self.hidden != e.hidden
            || self.kept != e.kept
    }

    /// Put edits back (undo, redo, a saved map opened). The map rebuilds
    /// on the next frame: sites, clients and the layout may have changed.
    pub(super) fn apply_edits(&mut self, e: &Edits) {
        self.placed.clone_from(&e.placed);
        self.mode = e.mode;
        self.clients = e.clients;
        self.reveal.clone_from(&e.reveal);
        self.closed.clone_from(&e.closed);
        self.hidden.clone_from(&e.hidden);
        self.kept.clone_from(&e.kept);
        // A pin put back takes the panel slot its card shows in.
        self.slots.retain(|s, _| e.kept.contains(s));
        for id in &e.kept {
            if !self.slots.contains_key(id) {
                let at = self.order.iter().position(|s| s == id).unwrap_or(0);
                self.slots.insert(id.clone(), at);
            }
        }
        self.stale = true;
    }

    /// After a rebuild: what it changed by itself (a pin on a card that's
    /// gone) isn't the user's. Mid-drag, the drag isn't over yet.
    pub(super) fn settle_edits(&mut self) {
        if self.drag.is_none() && self.edits_differ(self.history.now()) {
            self.history.settle(self.edits());
        }
    }

    /// Once a frame, after the user's input: anything changed since the
    /// last step is the next one. A drag counts once it's dropped.
    pub(super) fn track_edits(&mut self) {
        // Re-layout (the toolbar's or the canvas menu's) is named for
        // itself; a card menu's Put back on the only card moved isn't one.
        let relaid = std::mem::take(&mut self.relaid);
        if self.drag.is_some() || !self.edits_differ(self.history.now()) {
            return;
        }
        let now = self.edits();
        let mut what = describe(self.history.now(), &now, &|id| self.card_name(id));
        if relaid && what.starts_with("put back ") {
            what = "Re-layout".into();
        }
        self.history.record(now, what);
    }

    /// A card's label for a step's name, or its id when it's not drawn.
    fn card_name(&self, id: &str) -> String {
        self.index(id)
            .map(|i| self.map.nodes[i].label.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| id.to_string())
    }

    /// Undo (or redo) a step, if there's one. Not mid-drag.
    pub(super) fn step(&mut self, back: bool) {
        if self.drag.is_some() {
            return;
        }
        let e = if back {
            self.history.undo()
        } else {
            self.history.redo()
        }
        .cloned();
        if let Some(e) = e {
            self.apply_edits(&e);
        }
    }

    /// Ctrl+Z undoes; Ctrl+Shift+Z and Ctrl+Y redo. Not while a text box
    /// has the keyboard: it has its own undo.
    pub(super) fn history_keys(&mut self, ctx: &egui::Context) {
        // A dialog or menu open over the map: the keys are its, not the
        // map's underneath.
        let covered =
            egui::Popup::is_any_open(ctx) || ctx.memory(|m| m.top_modal_layer().is_some());
        if ctx.text_edit_focused() || covered {
            return;
        }
        use egui::{Key, Modifiers};
        let (redo, undo) = ctx.input_mut(|i| {
            // Most specific first: Ctrl+Z would match Ctrl+Shift+Z too.
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                || i.consume_key(Modifiers::COMMAND, Key::Y);
            (redo, i.consume_key(Modifiers::COMMAND, Key::Z))
        });
        if undo {
            self.step(true);
        } else if redo {
            self.step(false);
        }
    }

    /// ↩ and ↪ in the toolbar, each naming what it would do.
    pub(super) fn history_buttons(&mut self, ui: &mut Ui) {
        let undo = self.history.undo_label().map(str::to_string);
        let redo = self.history.redo_label().map(str::to_string);
        if ui
            .add_enabled(undo.is_some(), egui::Button::new("↩"))
            .on_hover_text(format!("Undo: {} (Ctrl+Z)", undo.unwrap_or_default()))
            .on_disabled_hover_text("Nothing to undo: moves, pins, folds and layouts can be undone")
            .clicked()
        {
            self.step(true);
        }
        if ui
            .add_enabled(redo.is_some(), egui::Button::new("↪"))
            .on_hover_text(format!(
                "Redo: {} (Ctrl+Shift+Z or Ctrl+Y)",
                redo.unwrap_or_default()
            ))
            .on_disabled_hover_text("Nothing to redo")
            .clicked()
        {
            self.step(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(id: &str, x: f32) -> Edits {
        let mut e = Edits::default();
        e.placed.insert(id.into(), pos2(x, 0.0));
        e
    }

    #[test]
    fn undo_and_redo_walk_the_steps() {
        let mut h: History<Edits> = History::default();
        h.record(moved("a", 1.0), "move a".into());
        h.record(moved("a", 2.0), "move a again".into());
        assert_eq!(h.undo_label(), Some("move a again"));
        assert_eq!(h.undo(), Some(&moved("a", 1.0)));
        assert_eq!(h.redo_label(), Some("move a again"));
        assert_eq!(h.undo(), Some(&Edits::default()));
        assert_eq!(h.undo(), None, "nothing before the first step");
        assert_eq!(h.redo(), Some(&moved("a", 1.0)));
        assert_eq!(h.redo(), Some(&moved("a", 2.0)));
        assert_eq!(h.redo(), None);
        assert_eq!(h.now(), &moved("a", 2.0));
    }

    #[test]
    fn a_new_step_clears_redo() {
        let mut h: History<Edits> = History::default();
        h.record(moved("a", 1.0), "one".into());
        h.record(moved("a", 2.0), "two".into());
        h.undo();
        assert_eq!(h.len(), (1, 1));
        h.record(moved("b", 3.0), "three".into());
        assert_eq!(h.len(), (2, 0));
        assert_eq!(h.redo_label(), None);
    }

    #[test]
    fn history_is_bounded() {
        let mut h: History<Edits> = History::default();
        for k in 0..STEPS + 30 {
            h.record(moved("a", k as f32), format!("step {k}"));
        }
        assert_eq!(h.len(), (STEPS, 0));
        // The oldest went first: undoing everything stops at step 30's
        // state before it, not at the start.
        while h.undo().is_some() {}
        assert_eq!(h.now(), &moved("a", 29.0));
        h.clear();
        assert_eq!(h.len(), (0, 0));
    }

    #[test]
    fn steps_are_named_for_what_changed() {
        let name = |id: &str| format!("<{id}>");
        let a = Edits::default();
        let mut b = moved("x", 1.0);
        assert_eq!(describe(&a, &b, &name), "move <x>");
        b.placed.insert("y".into(), pos2(2.0, 2.0));
        b.placed.insert("z".into(), pos2(2.0, 2.0));
        assert_eq!(describe(&a, &b, &name), "move 3 cards");
        assert_eq!(describe(&b, &a, &name), "put back 3 cards");
        let mut c = b.clone();
        c.placed.remove("y");
        assert_eq!(describe(&b, &c, &name), "put back <y>");
        let mut d = a.clone();
        d.mode = Some(Mode::Tree);
        d.placed = b.placed.clone();
        assert_eq!(describe(&b, &d, &name), "switch to the Tree layout");
        let mut e = a.clone();
        e.kept = vec!["x".into()];
        assert_eq!(describe(&a, &e, &name), "pin <x>");
        assert_eq!(describe(&e, &a, &name), "unpin <x>");
        let mut f = a.clone();
        f.closed.insert("Branch".into());
        assert_eq!(describe(&a, &f, &name), "collapse Branch");
        assert_eq!(describe(&f, &a, &name), "expand Branch");
        let mut g = a.clone();
        g.clients = Clients::Some;
        assert_eq!(describe(&a, &g, &name), "Clients: Some");
        let mut z = a.clone();
        z.hidden.insert(Zone::Server);
        assert_eq!(describe(&a, &z, &name), "fade out Servers / NAS");
        // Clients shown a few at a time on one card, then all, then hidden.
        let mut r = a.clone();
        r.reveal.insert("sw".into(), crate::map::STEP);
        assert_eq!(describe(&a, &r, &name), "show clients under <sw>");
        let mut all = a.clone();
        all.reveal.insert("sw".into(), usize::MAX);
        assert_eq!(describe(&r, &all, &name), "show all clients under <sw>");
        assert_eq!(describe(&all, &a, &name), "hide clients under <sw>");
    }

    /// +N on a card is a step: undo puts the card back as it was.
    #[test]
    fn a_reveal_step_is_undone() {
        let mut v = tree();
        v.track_edits();
        v.reveal_step(0, Some(crate::map::STEP));
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("show clients under Card 0"));
        v.step(true);
        assert!(v.reveal.is_empty());
        v.step(false);
        assert_eq!(v.reveal.get("n0"), Some(&crate::map::STEP));
    }

    fn tree() -> MapView {
        let mut v = MapView::new(None);
        v.map.nodes = (0..4)
            .map(|i| Node {
                id: format!("n{i}"),
                label: format!("Card {i}"),
                up: (i > 0).then_some((0, Link::Wired, None)),
                ..Default::default()
            })
            .collect();
        v.lay = layout::layout_in(&v.map, &|_| true, Mode::Cluster);
        v
    }

    #[test]
    fn a_drag_is_one_step_however_many_frames_it_takes() {
        let mut v = tree();
        v.drag = Some("n1".into());
        v.raised = vec!["n1".into(), "n2".into()];
        for f in 1..=20 {
            for id in v.raised.clone() {
                v.placed.insert(id, pos2(f as f32, 0.0));
            }
            v.track_edits();
            v.settle_edits();
        }
        assert_eq!(v.history.len(), (0, 0), "nothing until it's dropped");
        v.drag = None;
        v.track_edits();
        assert_eq!(v.history.len(), (1, 0));
        assert_eq!(v.history.undo_label(), Some("move 2 cards"));
        v.step(true);
        assert!(v.placed.is_empty(), "the whole drag undone");
        v.step(false);
        assert_eq!(v.placed.get("n2"), Some(&pos2(20.0, 0.0)));
    }

    /// Putting back the only card moved is "put back" it, as on several;
    /// Re-layout is named for itself however many cards it puts back.
    #[test]
    fn put_back_is_named_for_the_card_and_re_layout_for_itself() {
        let mut v = tree();
        v.placed.insert("n1".into(), pos2(5.0, 5.0));
        v.track_edits();
        v.placed.remove("n1");
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("put back Card 1"));
        v.step(true);
        v.placed.clear();
        v.relaid = true;
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("Re-layout"));
        assert!(!v.relaid, "only that frame's step");
    }

    #[test]
    fn selecting_isnt_a_step_but_pinning_is() {
        let mut v = tree();
        v.select(Some(1));
        v.toggle(2);
        v.track_edits();
        assert_eq!(v.history.len(), (0, 0));
        v.keep(2);
        v.track_edits();
        assert_eq!(v.history.undo_label(), Some("pin Card 2"));
        v.step(true);
        assert!(v.kept.is_empty() && v.slots.is_empty());
        v.step(false);
        assert_eq!(v.kept, ["n2"]);
        assert!(v.slots.contains_key("n2"));
    }

    #[test]
    fn what_a_rebuild_changes_isnt_a_step() {
        let mut v = tree();
        v.kept = vec!["n3".into()];
        v.track_edits();
        assert_eq!(v.history.len(), (1, 0));
        // n3 left the data: the rebuild drops its pin by itself.
        v.kept.clear();
        v.settle_edits();
        v.track_edits();
        assert_eq!(v.history.len(), (1, 0));
    }

    #[test]
    fn undo_puts_back_folds_and_layouts_and_rebuilds() {
        let mut v = tree();
        v.closed.insert("Branch".into());
        v.track_edits();
        v.mode = Some(Mode::Tree);
        v.track_edits();
        v.stale = false;
        v.step(true);
        assert_eq!(v.mode, None);
        assert!(v.stale, "rebuilt on the next frame");
        v.step(true);
        assert!(v.closed.is_empty());
        assert_eq!(v.history.redo_label(), Some("collapse Branch"));
    }
}
