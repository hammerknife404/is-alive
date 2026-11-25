//! Copy and paste across the app: every copy goes one way (marked to stay
//! out of clipboard history, cleared on exit; see `widgets::put`), selected
//! text in labels survives the right-click that opens its menu, and the
//! right-click menus of text boxes and values agree on what they offer.

use eframe::egui::{
    self, Context, Event, FullOutput, Id, OutputCommand, PointerButton, Response, Ui, ViewportId,
    text_selection::LabelSelectionState,
};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// The clipboard the app holds open on Linux (X11 serves a copy from the
/// program that made it).
#[cfg(all(unix, not(target_os = "macos")))]
pub fn held() -> MutexGuard<'static, Option<arboard::Clipboard>> {
    static CLIP: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
    CLIP.lock().unwrap_or_else(|e| e.into_inner())
}

/// Copies `copy` handed to the window's own clipboard (no marked one to
/// be had): the router lets these through rather than route them again.
static PASSED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub fn pass_through(text: &str) {
    lock(&PASSED).push(hash(text));
}

/// The selected label text a menu opened by the last right-click may
/// offer: only when that click landed on a label holding the selection
/// (empty otherwise, or when nothing is selected).
pub fn selected(ctx: &Context) -> String {
    let vp = ctx.viewport_id();
    ctx.plugin_opt::<Router>()
        .and_then(|r| {
            r.lock()
                .views
                .get(&vp)
                .filter(|v| v.on_selection)
                .map(|v| v.text.clone())
        })
        .unwrap_or_default()
}

/// Set up copying for `ctx`: once, at startup.
pub fn install(ctx: &Context) {
    ctx.add_plugin(Router::default());
}

/// Where a copy goes once routed: `widgets::put`, or a test's own.
type Sink = fn(String) -> Option<String>;

/// Sees every pass's input and output:
/// - every copy (Ctrl+C in a text box or on selected label text, a menu's
///   Copy, a Copy button) leaves through `widgets::put`, so all of them are
///   kept out of clipboard history and cleared on exit alike. This is done
///   on the pass's output, after egui's label selection has made its copy:
///   routing at the end of the app's own drawing missed that one;
/// - once after any input while labels hold a selection, it asks them for
///   the selected text (as Ctrl+C would) and keeps the answer, so a
///   right-click menu has it to offer after the click itself has moved
///   egui's selection; and it puts the selection back after a right-click
///   on the label that holds it.
///
/// It asks only after input, never on an idle frame, and pushes its
/// question into the pass after egui has decided whether to redraw: an
/// idle window with text selected doesn't redraw again for it.
struct Router {
    /// Each window's state: the manual in its own window has its own.
    views: HashMap<ViewportId, View>,
    /// The windows whose passes are under way, innermost last.
    passes: Vec<ViewportId>,
    sink: Sink,
}

impl Default for Router {
    fn default() -> Self {
        Self {
            views: HashMap::new(),
            passes: Vec::new(),
            sink: crate::widgets::put,
        }
    }
}

#[derive(Default)]
struct View {
    /// The label selection as it was when last asked.
    saved: Option<LabelSelectionState>,
    /// The labels holding the selection's two ends, as egui's debug text
    /// names them (`selection_ends`).
    ends: Vec<String>,
    /// The selected text, as last asked.
    text: String,
    /// Input arrived since the labels were last asked.
    dirty: bool,
    /// This pass asks the labels for their selected text, not the user.
    asking: bool,
    /// A right-click on the selection is under way: put it back until it's
    /// released, and once more after.
    right: Option<bool>,
    /// The last right-click landed on a label holding the selection.
    on_selection: bool,
    /// A right-click on the selection just ended: open its menu.
    menu: bool,
    /// Copy all was picked in this block's menu: its owner copies.
    copy_all: Option<Id>,
}

/// The labels at the selection's two ends, by the short form of their
/// ids. egui keeps them private; its debug text names them, and
/// `a_selections_ends_are_named_by_the_labels_holding_them` pins that.
fn selection_ends(labels: &LabelSelectionState) -> Vec<String> {
    let text = format!("{labels:?}");
    let key = "widget_id: \"";
    text.match_indices(key)
        .filter_map(|(at, _)| text.get(at + key.len()..at + key.len() + 4))
        .map(str::to_string)
        .collect()
}

impl egui::Plugin for Router {
    fn debug_name(&self) -> &'static str {
        "is-alive copy router"
    }

    // Runs after egui's own label selection has begun the pass (it was
    // registered first), before anything is drawn.
    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let vp = ctx.viewport_id();
        self.passes.push(vp);
        let (any, quiet, pressed, released) = ctx.input(|i| {
            let (p, r) = right_events(&i.events);
            (!i.events.is_empty(), quiet(&i.events), p, r)
        });
        let handle = ctx.plugin::<LabelSelectionState>();
        let mut labels = handle.lock();
        let v = self.views.entry(vp).or_default();
        v.dirty |= any;
        let mut menu = false;
        if let Some(released) = v.right {
            if let Some(s) = &v.saved {
                *labels = s.clone();
            }
            if released {
                menu = true;
                v.right = None;
            }
        }
        if pressed {
            // On the selection: a label holding one of its ends.
            let under: Vec<String> = ctx.interaction_snapshot(|s| {
                s.hovered.iter().map(|id| id.short_debug_format()).collect()
            });
            v.on_selection =
                !v.text.is_empty() && v.saved.is_some() && under.iter().any(|u| v.ends.contains(u));
            if v.on_selection {
                v.right = Some(false);
            }
        }
        if released && v.right.is_some() {
            v.right = Some(true);
        }
        v.menu = menu;
        let has = labels.has_selection();
        // Its own Ctrl+C is a focused text box's.
        let focused = ctx.memory(|m| m.focused().is_some());
        v.asking = has && v.dirty && quiet && v.right.is_none() && !focused;
        if v.asking {
            v.dirty = false;
            v.saved = Some(labels.clone());
            v.ends = selection_ends(&labels);
            drop(labels);
            ctx.input_mut(|i| i.events.push(Event::Copy));
        } else if !has && v.right.is_none() && !menu && !egui::Popup::is_any_open(&ctx) {
            v.saved = None;
            v.ends.clear();
            v.text.clear();
            v.on_selection = false;
        }
    }

    fn output_hook(&mut self, _ctx: &Context, output: &mut FullOutput) {
        let vp = self.passes.pop().unwrap_or(ViewportId::ROOT);
        let mut texts = Vec::new();
        let mut passed = lock(&PASSED);
        output.platform_output.commands.retain(|c| match c {
            OutputCommand::CopyText(t) => {
                if let Some(at) = passed.iter().position(|h| *h == hash(t)) {
                    passed.swap_remove(at);
                    return true;
                }
                texts.push(t.clone());
                false
            }
            _ => true,
        });
        passed.clear();
        drop(passed);
        if let Some(v) = self.views.get_mut(&vp)
            && std::mem::take(&mut v.asking)
        {
            v.text = texts.concat();
            return;
        }
        for t in texts {
            if let Some(t) = (self.sink)(t) {
                output
                    .platform_output
                    .commands
                    .push(OutputCommand::CopyText(t));
            }
        }
    }
}

/// No click, no key, nothing typed or pasted: the labels' selection is
/// settled, and a Copy asked now is only the router's.
fn quiet(events: &[Event]) -> bool {
    !events.iter().any(|e| {
        matches!(
            e,
            Event::PointerButton { .. }
                | Event::Key { .. }
                | Event::Copy
                | Event::Cut
                | Event::Paste(_)
                | Event::Text(_)
        )
    })
}

/// Whether `events` press and release the right button.
fn right_events(events: &[Event]) -> (bool, bool) {
    let mut out = (false, false);
    for e in events {
        if let Event::PointerButton {
            button: PointerButton::Secondary,
            pressed,
            ..
        } = e
        {
            if *pressed {
                out.0 = true;
            } else {
                out.1 = true;
            }
        }
    }
    out
}

/// Mark `rect`, drawn in `ui`, as a block of text (a preview made of many
/// labels) known as `key`: a right-click in it opens the label menu with
/// Copy all as well. The text isn't handed over: when Copy all is picked,
/// `take_copy_all` says so to the block's owner, which copies its own.
pub fn block(ui: &Ui, rect: egui::Rect, key: Id) {
    let layer = ui.layer_id();
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(egui::Rect, egui::LayerId, Id)>>(Id::new("clip blocks"))
            .push((rect, layer, key))
    });
}

/// Whether Copy all was picked in block `key`'s menu (once).
pub fn take_copy_all(ctx: &Context, key: Id) -> bool {
    let vp = ctx.viewport_id();
    ctx.plugin_opt::<Router>().is_some_and(|r| {
        r.lock()
            .views
            .get_mut(&vp)
            .is_some_and(|v| v.copy_all.take_if(|k| *k == key).is_some())
    })
}

/// The right-click menu for text in labels: Copy selected text (when the
/// right-click landed on it) and, in a `block`, Copy all. Not when what
/// was clicked opened a menu of its own. Call once a pass, after the UI,
/// in each window.
pub fn label_menu(ctx: &Context) {
    let id = Id::new("clip label menu");
    let blocks: Vec<(egui::Rect, egui::LayerId, Id)> = ctx
        .data_mut(|d| d.remove_temp(Id::new("clip blocks")))
        .unwrap_or_default();
    let Some(router) = ctx.plugin_opt::<Router>() else {
        return;
    };
    let vp = ctx.viewport_id();
    let on_selection = router
        .lock()
        .views
        .get_mut(&vp)
        .is_some_and(|v| std::mem::take(&mut v.menu))
        && !selected(ctx).is_empty();
    let pointer = ctx.pointer_latest_pos();
    // In a block, and not under a window drawn over it.
    let in_block = ctx
        .input(|i| i.pointer.secondary_clicked())
        .then_some(pointer)
        .flatten()
        .and_then(|p| {
            let top = ctx.layer_id_at(p);
            blocks
                .iter()
                .rev()
                .find(|(r, layer, _)| r.contains(p) && top == Some(*layer))
        })
        .map(|(.., key)| *key);
    let open = (on_selection || in_block.is_some()) && !egui::Popup::is_any_open(ctx);
    if open && let Some(p) = pointer {
        ctx.data_mut(|d| d.insert_temp(id, (p, in_block)));
    }
    let Some((at, block)) = ctx.data(|d| d.get_temp::<(egui::Pos2, Option<Id>)>(id)) else {
        return;
    };
    let shown = egui::Popup::new(
        id,
        ctx.clone(),
        egui::PopupAnchor::Position(at),
        egui::LayerId::new(egui::Order::Foreground, id),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .style(egui::containers::menu::menu_style)
    .open_memory(open.then_some(egui::SetOpenCommand::Bool(true)))
    .show(|ui| {
        let sel = selected(ui.ctx());
        if !sel.is_empty()
            && ui
                .add(egui::Button::new(COPY_SELECTED).shortcut_text(shortcut("C")))
                .clicked()
        {
            crate::widgets::copy(ui.ctx(), sel);
            ui.close();
        }
        if let Some(key) = block
            && ui.button(COPY_ALL).clicked()
        {
            if let Some(r) = ui.ctx().plugin_opt::<Router>() {
                r.lock().views.entry(vp).or_default().copy_all = Some(key);
            }
            ui.ctx().request_repaint();
            ui.close();
        }
    });
    if shown.is_none() {
        ctx.data_mut(|d| d.remove::<(egui::Pos2, Option<Id>)>(id));
    }
}

/// The menus' words: what's selected, a value whole, a block whole.
pub const COPY_SELECTED: &str = "Copy selected text";
pub const COPY: &str = "Copy";
pub const COPY_ALL: &str = "Copy all";

/// A key's shortcut as this system writes it.
pub fn shortcut(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// What a text box's right-click menu offers, and which of it works now.
/// `None`: not offered; `Some(false)`: offered but greyed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Items {
    pub cut: Option<bool>,
    pub copy: Option<bool>,
    pub paste: Option<bool>,
    pub select_all: Option<bool>,
}

/// How a text box can be used.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct TextBox {
    /// It can be typed in (not a read-only view).
    pub editable: bool,
    /// It hides what's in it (a password, a community string).
    pub masked: bool,
}

pub const EDITABLE: TextBox = TextBox {
    editable: true,
    masked: false,
};
pub const READ_ONLY: TextBox = TextBox {
    editable: false,
    masked: false,
};
pub const MASKED: TextBox = TextBox {
    editable: true,
    masked: true,
};

/// The menu for a box like `b`, with text `selected` in it or not, and the
/// clipboard holding text (`Some(true)`), not (`Some(false)`) or unknown
/// (`None`: Paste stays on, and pastes nothing if there's nothing).
/// A masked box never gives its text away: no Cut, no Copy.
pub fn items(b: TextBox, selected: bool, clipboard: Option<bool>, empty: bool) -> Items {
    Items {
        cut: (b.editable && !b.masked).then_some(selected),
        copy: (!b.masked).then_some(selected),
        paste: b.editable.then_some(clipboard != Some(false)),
        select_all: Some(!empty),
    }
}

/// Whether the clipboard holds text, for menu `id`: read once as the menu
/// opens and kept while it stays open (a menu is drawn every pass).
/// `None` when it can't be read (another program holding it): unknown.
pub fn clipboard_has_text(ctx: &Context, id: Id) -> Option<bool> {
    let key = id.with("clip paste");
    let pass = ctx.cumulative_pass_nr();
    let kept = ctx.data(|d| d.get_temp::<(u64, Option<bool>)>(key));
    let v = match kept {
        // Still open: drawn on the pass before this one.
        Some((at, v)) if at + 1 >= pass => v,
        _ => read_has_text(),
    };
    ctx.data_mut(|d| d.insert_temp(key, (pass, v)));
    v
}

fn read_has_text() -> Option<bool> {
    #[cfg(all(unix, not(target_os = "macos")))]
    let got = {
        let mut held = held();
        if held.is_none() {
            *held = arboard::Clipboard::new().ok();
        }
        held.as_mut()?.get_text()
    };
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let got = arboard::Clipboard::new().ok()?.get_text();
    has_text(got)
}

/// What a clipboard read says: text, none, or (any other error) unknown.
fn has_text(got: Result<String, arboard::Error>) -> Option<bool> {
    match got {
        Ok(t) => Some(!t.is_empty()),
        Err(arboard::Error::ContentNotAvailable) => Some(false),
        Err(_) => None,
    }
}

/// Give `r` (a value: an address, a name, a finding) a right-click menu:
/// Copy selected text, when the right-click landed on a selection in it,
/// and Copy, for `value` whole.
pub fn value_menu(r: &Response, value: &str) {
    r.context_menu(|ui| value_items(ui, value));
}

/// For a block of text in one label (a run's output): Copy selected
/// text, as `value_menu`, and Copy all of `text`.
pub fn block_menu(r: &Response, text: &str) {
    r.context_menu(|ui| copy_items(ui, text, COPY_ALL));
}

/// `value_menu`'s items, for a menu that has others too.
pub fn value_items(ui: &mut Ui, value: &str) {
    copy_items(ui, value, COPY);
}

fn copy_items(ui: &mut Ui, value: &str, whole: &str) {
    let sel = selected(ui.ctx());
    if !sel.is_empty()
        && sel != value
        && ui
            .add(egui::Button::new(COPY_SELECTED).shortcut_text(shortcut("C")))
            .clicked()
    {
        crate::widgets::copy(ui.ctx(), sel);
        ui.close();
    }
    let tip = if value.chars().count() > 40 {
        format!("{}…", value.chars().take(40).collect::<String>())
    } else {
        value.to_string()
    };
    if ui
        .add(egui::Button::new(whole))
        .on_hover_text(tip)
        .clicked()
    {
        crate::widgets::copy(ui.ctx(), value.to_string());
        crate::widgets::toast("Copied to the clipboard", true);
        ui.close();
    }
}

/// A value as a selectable label with `value_menu`.
pub fn value(ui: &mut Ui, text: impl Into<egui::WidgetText>, value: &str) -> Response {
    let r = ui.add(egui::Label::new(text).selectable(true));
    value_menu(&r, value);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boxs_menu_offers_what_its_state_allows() {
        // An editable box, text selected, text on the clipboard: all of it.
        let all = items(EDITABLE, true, Some(true), false);
        assert_eq!(
            all,
            Items {
                cut: Some(true),
                copy: Some(true),
                paste: Some(true),
                select_all: Some(true),
            }
        );
        // Nothing selected: Cut and Copy greyed, Paste and Select all on.
        let none = items(EDITABLE, false, Some(true), false);
        assert_eq!((none.cut, none.copy), (Some(false), Some(false)));
        assert_eq!(none.paste, Some(true));
        // Nothing on the clipboard: Paste greyed. Unknown: left on.
        assert_eq!(items(EDITABLE, true, Some(false), false).paste, Some(false));
        assert_eq!(items(EDITABLE, true, None, false).paste, Some(true));
        // An empty box: nothing to select.
        assert_eq!(items(EDITABLE, false, None, true).select_all, Some(false));
    }

    #[test]
    fn a_read_only_view_offers_no_cut_or_paste() {
        let r = items(READ_ONLY, true, Some(true), false);
        assert_eq!(r.cut, None);
        assert_eq!(r.paste, None);
        assert_eq!(r.copy, Some(true));
        assert_eq!(r.select_all, Some(true));
    }

    #[test]
    fn a_masked_box_never_offers_its_text() {
        for sel in [true, false] {
            for clip in [Some(true), Some(false), None] {
                let m = items(MASKED, sel, clip, false);
                assert_eq!(m.cut, None, "no Cut");
                assert_eq!(m.copy, None, "no Copy");
                // Typing a community string in by paste still works.
                assert!(m.paste.is_some());
            }
        }
    }

    #[test]
    fn right_button_events_are_told_apart() {
        let ev = |button, pressed| Event::PointerButton {
            pos: egui::Pos2::ZERO,
            button,
            pressed,
            modifiers: Default::default(),
        };
        assert_eq!(right_events(&[]), (false, false));
        assert_eq!(
            right_events(&[ev(PointerButton::Primary, true)]),
            (false, false)
        );
        assert_eq!(
            right_events(&[
                ev(PointerButton::Secondary, true),
                ev(PointerButton::Secondary, false)
            ]),
            (true, true)
        );
    }

    /// A window to drive in a test: the router installed, copies it
    /// routes kept in `ROUTED` (this thread's), each pass's UI given by
    /// `ui`, which returns the rects it wants to click on.
    struct Rig {
        ctx: Context,
    }

    thread_local! {
        static ROUTED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn keep(t: String) -> Option<String> {
        ROUTED.with(|r| r.borrow_mut().push(t));
        None
    }

    fn routed() -> Vec<String> {
        ROUTED.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }

    impl Rig {
        fn new() -> Self {
            let ctx = Context::default();
            ctx.add_plugin(Router {
                sink: keep,
                ..Default::default()
            });
            Rig { ctx }
        }

        /// One pass with `events`; its output.
        fn pass(&self, events: Vec<Event>, ui: &mut dyn FnMut(&mut Ui)) -> FullOutput {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = self.ctx.run_ui(input, |u| {
                ui(u);
                label_menu(u.ctx());
            });
            out.textures_delta.clear();
            out
        }

        fn view<R>(&self, f: impl FnOnce(&View) -> R) -> R {
            let r = self.ctx.plugin::<Router>();
            let r = r.lock();
            f(&r.views[&ViewportId::ROOT])
        }
    }

    fn button(pos: egui::Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        }
    }

    /// Drag across `r` with the left button, then let the passes settle.
    fn drag_over(rig: &Rig, r: egui::Rect, ui: &mut dyn FnMut(&mut Ui)) {
        let (a, b) = (r.left_center() + egui::vec2(1.0, 0.0), r.right_center());
        rig.pass(
            vec![
                Event::PointerMoved(a),
                button(a, PointerButton::Primary, true),
            ],
            ui,
        );
        rig.pass(vec![Event::PointerMoved(r.center())], ui);
        rig.pass(vec![Event::PointerMoved(b)], ui);
        rig.pass(vec![button(b, PointerButton::Primary, false)], ui);
        rig.pass(vec![], ui);
        rig.pass(vec![], ui);
    }

    fn right_click(rig: &Rig, at: egui::Pos2, ui: &mut dyn FnMut(&mut Ui)) {
        rig.pass(
            vec![
                Event::PointerMoved(at),
                button(at, PointerButton::Secondary, true),
            ],
            ui,
        );
        rig.pass(vec![button(at, PointerButton::Secondary, false)], ui);
        rig.pass(vec![], ui);
        rig.pass(vec![], ui);
    }

    const LABEL_MENU: &str = "clip label menu";

    /// Text dragged over in a label is still there after the right-click
    /// that opens its menu, which offers Copy selected text.
    #[test]
    fn selected_label_text_survives_a_right_click() {
        let rig = Rig::new();
        let text = "Gateway 192.0.2.1 answers";
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut ui = |ui: &mut Ui| rect.set(ui.label(text).rect);
        rig.pass(vec![], &mut ui);
        drag_over(&rig, rect.get(), &mut ui);
        assert_eq!(rig.view(|v| v.text.clone()), text);
        assert!(routed().is_empty(), "asking copies nothing");
        right_click(&rig, rect.get().center(), &mut ui);
        assert!(egui::Popup::is_id_open(&rig.ctx, Id::new(LABEL_MENU)));
        assert_eq!(selected(&rig.ctx), text);
        assert!(
            rig.ctx
                .plugin::<LabelSelectionState>()
                .lock()
                .has_selection()
        );
        // Esc shuts the menu and lets the selection go.
        let esc = Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        rig.pass(vec![esc], &mut ui);
        rig.pass(vec![], &mut ui);
        rig.pass(vec![], &mut ui);
        assert!(!egui::Popup::is_any_open(&rig.ctx));
    }

    /// The leak this fixes: Ctrl+C on selected label text is copied by
    /// egui's label selection after the app has drawn, and went to the
    /// window's clipboard unmarked. It goes through the router now.
    #[test]
    fn ctrl_c_on_selected_label_text_goes_through_the_router() {
        let rig = Rig::new();
        let text = "Serial EXAMPLE0123";
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut ui = |ui: &mut Ui| rect.set(ui.label(text).rect);
        rig.pass(vec![], &mut ui);
        drag_over(&rig, rect.get(), &mut ui);
        routed();
        // Ctrl+C as the window delivers it: the key, and Copy.
        let ctrl_c = Event::Key {
            key: egui::Key::C,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let out = rig.pass(vec![ctrl_c, Event::Copy], &mut ui);
        assert_eq!(routed(), [text]);
        assert!(
            !out.platform_output
                .commands
                .iter()
                .any(|c| matches!(c, OutputCommand::CopyText(_))),
            "nothing left for the window to copy unmarked"
        );
    }

    /// A window with text selected and nothing happening doesn't redraw:
    /// the router asks the labels once after input, then leaves it.
    #[test]
    fn an_idle_window_with_a_selection_doesnt_redraw() {
        let rig = Rig::new();
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut ui = |ui: &mut Ui| rect.set(ui.label("Uptime 12 d 4 h").rect);
        rig.pass(vec![], &mut ui);
        drag_over(&rig, rect.get(), &mut ui);
        assert!(!rig.view(|v| v.text.is_empty()));
        for _ in 0..3 {
            let out = rig.pass(vec![], &mut ui);
            let delay = out.viewport_output[&ViewportId::ROOT].repaint_delay;
            assert_eq!(delay, std::time::Duration::MAX, "idle");
        }
        assert!(
            rig.ctx
                .plugin::<LabelSelectionState>()
                .lock()
                .has_selection()
        );
    }

    /// Right-clicking other text while some is selected doesn't offer the
    /// selection, or put it back: the click is about what it landed on.
    #[test]
    fn a_right_click_on_other_text_leaves_the_selection_be() {
        let rig = Rig::new();
        let rects = std::cell::Cell::new((egui::Rect::NOTHING, egui::Rect::NOTHING));
        let mut ui = |ui: &mut Ui| {
            let a = ui.label("MAC from the device").rect;
            ui.add_space(40.0);
            let b = ui.label("Vendor Synology").rect;
            rects.set((a, b));
        };
        rig.pass(vec![], &mut ui);
        drag_over(&rig, rects.get().0, &mut ui);
        assert!(!rig.view(|v| v.text.is_empty()));
        right_click(&rig, rects.get().1.center(), &mut ui);
        assert!(!egui::Popup::is_id_open(&rig.ctx, Id::new(LABEL_MENU)));
        assert_eq!(selected(&rig.ctx), "");
    }

    /// With text selected in a side panel, a canvas beside it (as the Map
    /// is) still pans on a drag, and a right-click on it opens its own
    /// menu, not the label's.
    #[test]
    fn a_selection_leaves_a_canvas_its_drags_and_its_menu() {
        let rig = Rig::new();
        let panned = std::cell::Cell::new(0.0_f32);
        let canvas_menu = std::cell::Cell::new(false);
        let label = std::cell::Cell::new(egui::Rect::NOTHING);
        let area = egui::Rect::from_min_size(egui::pos2(300.0, 100.0), egui::vec2(300.0, 300.0));
        let mut ui = |ui: &mut Ui| {
            label.set(ui.label("Switch 198.51.100.4").rect);
            let canvas = ui.interact(area, Id::new("canvas"), egui::Sense::click_and_drag());
            panned.set(panned.get() + canvas.drag_delta().x);
            canvas.context_menu(|ui| {
                canvas_menu.set(true);
                ui.label("Card menu");
            });
        };
        rig.pass(vec![], &mut ui);
        drag_over(&rig, label.get(), &mut ui);
        assert!(!rig.view(|v| v.text.is_empty()));
        // A drag on the canvas pans it.
        let c = area.center();
        rig.pass(
            vec![
                Event::PointerMoved(c),
                button(c, PointerButton::Primary, true),
            ],
            &mut ui,
        );
        for dx in 1..=6 {
            rig.pass(
                vec![Event::PointerMoved(c + egui::vec2(10.0 * dx as f32, 0.0))],
                &mut ui,
            );
        }
        rig.pass(
            vec![button(
                c + egui::vec2(60.0, 0.0),
                PointerButton::Primary,
                false,
            )],
            &mut ui,
        );
        assert!(panned.get() > 30.0, "panned {}", panned.get());
        // Select again, then right-click the canvas: its own menu.
        drag_over(&rig, label.get(), &mut ui);
        right_click(&rig, c, &mut ui);
        assert!(canvas_menu.get(), "the canvas's menu");
        assert!(!egui::Popup::is_id_open(&rig.ctx, Id::new(LABEL_MENU)));
    }

    /// A block's Copy all is its owner's to copy, asked only when picked;
    /// a right-click on a window drawn over the block isn't in the block.
    #[test]
    fn a_blocks_copy_all_is_left_to_its_owner() {
        let rig = Rig::new();
        let key = Id::new("preview");
        let block = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0));
        let over = std::cell::Cell::new(false);
        let mut ui = |ui: &mut Ui| {
            ui.label("Notes in Markdown");
            super::block(ui, block, key);
            if over.get() {
                egui::Area::new(Id::new("window over"))
                    .fixed_pos(egui::pos2(100.0, 100.0))
                    .show(ui.ctx(), |ui| {
                        ui.allocate_exact_size(egui::vec2(200.0, 150.0), egui::Sense::click());
                    });
            }
        };
        rig.pass(vec![], &mut ui);
        let at = egui::pos2(150.0, 150.0);
        right_click(&rig, at, &mut ui);
        assert!(egui::Popup::is_id_open(&rig.ctx, Id::new(LABEL_MENU)));
        assert!(!take_copy_all(&rig.ctx, key), "not picked yet");
        // Over another window: no block menu.
        let esc = Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        rig.pass(vec![esc], &mut ui);
        over.set(true);
        rig.pass(vec![], &mut ui);
        right_click(&rig, at, &mut ui);
        assert!(!egui::Popup::is_id_open(&rig.ctx, Id::new(LABEL_MENU)));
    }

    /// `selection_ends` reads egui's private selection from its debug
    /// text: this pins that it names the label holding the selection.
    #[test]
    fn a_selections_ends_are_named_by_the_labels_holding_them() {
        let rig = Rig::new();
        let id = std::cell::Cell::new(Id::NULL);
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut ui = |ui: &mut Ui| {
            let r = ui.label("Firmware 20231115");
            id.set(r.id);
            rect.set(r.rect);
        };
        rig.pass(vec![], &mut ui);
        drag_over(&rig, rect.get(), &mut ui);
        let ends = selection_ends(&rig.ctx.plugin::<LabelSelectionState>().lock());
        assert!(!ends.is_empty());
        assert!(
            ends.iter().all(|e| *e == id.get().short_debug_format()),
            "{ends:?}"
        );
    }

    #[test]
    fn a_clipboard_that_cant_be_read_leaves_paste_on() {
        assert_eq!(has_text(Ok("192.0.2.1".into())), Some(true));
        assert_eq!(has_text(Ok(String::new())), Some(false));
        assert_eq!(
            has_text(Err(arboard::Error::ContentNotAvailable)),
            Some(false)
        );
        // Held by another program (a remote-desktop session): unknown.
        assert_eq!(has_text(Err(arboard::Error::ClipboardOccupied)), None);
        assert_eq!(items(EDITABLE, false, None, false).paste, Some(true));
    }
}
