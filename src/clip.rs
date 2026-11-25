//! Copy and paste across the app: every copy goes one way (marked to stay
//! out of clipboard history, cleared on exit; see `widgets::put`), selected
//! text in labels survives the right-click that opens its menu, and the
//! right-click menus of text boxes and values agree on what they offer.

use eframe::egui::{
    self, Context, Event, FullOutput, Id, OutputCommand, PointerButton, RawInput, Response, Ui,
    text_selection::LabelSelectionState,
};
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

/// The text selected in labels (empty when none).
pub fn selected(ctx: &Context) -> String {
    ctx.plugin_opt::<Router>()
        .map(|r| r.lock().text.clone())
        .unwrap_or_default()
}

/// Set up copying for `ctx`: once, at startup.
pub fn install(ctx: &Context) {
    ctx.add_plugin(Router::default());
}

/// Sees every frame's input and output:
/// - every copy (Ctrl+C in a text box or on selected label text, a menu's
///   Copy, a Copy button) leaves through `widgets::put`, so all of them are
///   kept out of clipboard history and cleared on exit alike;
/// - while labels hold a selection, it notes the selected text each quiet
///   frame (`text`), and puts the selection back after a right-click on
///   a label moved it, so the menu that click opens has it to copy.
#[derive(Default)]
struct Router {
    /// The label selection as of the last quiet frame.
    saved: Option<LabelSelectionState>,
    /// This frame asks the labels for their selected text, not the user.
    asking: bool,
    /// A right-click is under way: pressed, not yet seen through.
    right: Option<Right>,
    /// A right-click on selected text just ended: open its menu.
    menu: bool,
    /// The text selected across labels, as of the last frame that had a
    /// selection and no click: what a right-click menu's Copy copies,
    /// after the click itself has moved egui's selection.
    text: String,
}

#[derive(Clone, Copy, Default)]
struct Right {
    /// Whether it landed on a label; known the frame after the press.
    on_label: Option<bool>,
    released: bool,
}

impl egui::Plugin for Router {
    fn debug_name(&self) -> &'static str {
        "is-alive copy router"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        let labels = ctx.plugin::<LabelSelectionState>();
        let mut labels = labels.lock();
        let has = labels.has_selection();
        let (pressed, released) = right_events(&input.events);
        let mut menu = false;
        if let Some(r) = self.right.as_mut() {
            // A press on a label left a selection there (a point, where it
            // landed); one anywhere else let it go.
            let on = *r.on_label.get_or_insert(has);
            if on && let Some(s) = &self.saved {
                *labels = s.clone();
            }
            if r.released {
                menu = on;
                self.right = None;
            }
        }
        if pressed && self.saved.is_some() && !self.text.is_empty() {
            self.right = Some(Right::default());
        }
        if released && let Some(r) = self.right.as_mut() {
            r.released = true;
        }
        self.menu = menu;
        let has = labels.has_selection();
        drop(labels);
        // A quiet frame: no click, no key, no text box focused (its own
        // Ctrl+C is the user's). Ask the labels what's selected, as Ctrl+C
        // would; the router keeps the answer rather than copy it.
        let quiet = !input.events.iter().any(|e| {
            matches!(
                e,
                Event::PointerButton { .. }
                    | Event::Key { .. }
                    | Event::Copy
                    | Event::Cut
                    | Event::Paste(_)
                    | Event::Text(_)
            )
        });
        let focused = ctx.memory(|m| m.focused().is_some());
        self.asking = has && quiet && self.right.is_none() && !focused;
        if self.asking {
            input.events.push(Event::Copy);
            self.saved = Some(ctx.plugin::<LabelSelectionState>().lock().clone());
        } else if !has && self.right.is_none() && !menu && !egui::Popup::is_any_open(ctx) {
            self.saved = None;
            self.text.clear();
        }
    }

    fn output_hook(&mut self, _ctx: &Context, output: &mut FullOutput) {
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
        if std::mem::take(&mut self.asking) {
            self.text = texts.concat();
            return;
        }
        for t in texts {
            if let Some(t) = crate::widgets::put(t) {
                output
                    .platform_output
                    .commands
                    .push(OutputCommand::CopyText(t));
            }
        }
    }
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

/// Mark `rect` as a block of text (a preview drawn from many labels) whose
/// whole is `text`: a right-click in it opens the label menu with Copy all
/// as well. Call each frame it's drawn.
pub fn block(ctx: &Context, rect: egui::Rect, text: &str) {
    let id = Id::new("clip blocks");
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(egui::Rect, String)>>(id)
            .push((rect, text.to_string()))
    });
}

/// The right-click menu for text in labels: Copy (what's selected) and,
/// in a `block`, Copy all. Opens where a right-click on selected text, or
/// in a block, ended; not when what was clicked opened a menu of its own.
/// Call once a frame, after the UI, in each window.
pub fn label_menu(ctx: &Context) {
    let id = Id::new("clip label menu");
    let blocks: Vec<(egui::Rect, String)> = ctx
        .data_mut(|d| d.remove_temp(Id::new("clip blocks")))
        .unwrap_or_default();
    let Some(router) = ctx.plugin_opt::<Router>() else {
        return;
    };
    let on_selection = std::mem::take(&mut router.lock().menu) && !selected(ctx).is_empty();
    let pointer = ctx.pointer_latest_pos();
    let in_block = ctx
        .input(|i| i.pointer.secondary_clicked())
        .then_some(pointer)
        .flatten()
        .and_then(|p| blocks.iter().rev().find(|(r, _)| r.contains(p)))
        .map(|(_, t)| t.clone());
    let open = (on_selection || in_block.is_some()) && !egui::Popup::is_any_open(ctx);
    if open && let Some(p) = pointer {
        ctx.data_mut(|d| d.insert_temp(id, (p, in_block.unwrap_or_default())));
    }
    let Some((at, all)) = ctx.data(|d| d.get_temp::<(egui::Pos2, String)>(id)) else {
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
        if ui
            .add_enabled(
                !sel.is_empty(),
                egui::Button::new("Copy").shortcut_text(shortcut("C")),
            )
            .clicked()
        {
            crate::widgets::copy(ui.ctx(), sel);
            ui.close();
        }
        if !all.is_empty() && ui.button("Copy all").clicked() {
            crate::widgets::copy(ui.ctx(), all.clone());
            crate::widgets::toast("Copied to the clipboard", true);
            ui.close();
        }
    });
    if shown.is_none() {
        ctx.data_mut(|d| d.remove::<(egui::Pos2, String)>(id));
    }
}

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

/// Whether the clipboard holds text; `None` when it can't be read here.
/// Asked at most twice a second, as a menu is redrawn every frame.
pub fn clipboard_has_text() -> Option<bool> {
    static ASKED: Mutex<Option<(std::time::Instant, Option<bool>)>> = Mutex::new(None);
    let mut asked = lock(&ASKED);
    if let Some((at, v)) = *asked
        && at.elapsed().as_secs_f32() < 0.5
    {
        return v;
    }
    let v = read_has_text();
    *asked = Some((std::time::Instant::now(), v));
    v
}

fn read_has_text() -> Option<bool> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut held = held();
        if held.is_none() {
            *held = arboard::Clipboard::new().ok();
        }
        let c = held.as_mut()?;
        Some(c.get_text().is_ok_and(|t| !t.is_empty()))
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let mut c = arboard::Clipboard::new().ok()?;
        Some(c.get_text().is_ok_and(|t| !t.is_empty()))
    }
}

/// Give `r` (a value: an address, a name, a finding) a right-click menu:
/// Copy the selected text when some is selected, and Copy `value` whole.
pub fn value_menu(r: &Response, value: &str) {
    r.context_menu(|ui| value_items(ui, value));
}

/// For a block of text (a run's output, a preview): Copy the selected
/// text when some is selected, and Copy all of `text`.
pub fn block_menu(r: &Response, text: &str) {
    r.context_menu(|ui| copy_items(ui, text, "Copy all"));
}

/// `value_menu`'s items, for a menu that has others too.
pub fn value_items(ui: &mut Ui, value: &str) {
    copy_items(ui, value, "Copy");
}

fn copy_items(ui: &mut Ui, value: &str, whole: &str) {
    let sel = selected(ui.ctx());
    if !sel.is_empty()
        && sel != value
        && ui
            .add(egui::Button::new("Copy selected text").shortcut_text(shortcut("C")))
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

    /// Text dragged over in a label is still there to copy after the
    /// right-click that opens its menu, which then offers Copy; and Ctrl+C
    /// on it leaves through the router, not straight to the window.
    #[test]
    fn selected_label_text_survives_a_right_click() {
        let ctx = egui::Context::default();
        install(&ctx);
        let text = "Gateway 192.0.2.1 answers";
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let frame = |events: Vec<Event>| {
            let input = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                rect.set(ui.label(text).rect);
                label_menu(ui.ctx());
            });
            out.textures_delta.clear();
            std::mem::take(&mut out.platform_output.commands)
        };
        let button = |pos, button, pressed| Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![]);
        let rect = rect.get();
        let (a, b) = (
            rect.left_center() + egui::vec2(1.0, 0.0),
            rect.right_center(),
        );
        // Drag across the whole label.
        frame(vec![
            Event::PointerMoved(a),
            button(a, PointerButton::Primary, true),
        ]);
        frame(vec![Event::PointerMoved(rect.center())]);
        frame(vec![Event::PointerMoved(b)]);
        frame(vec![button(b, PointerButton::Primary, false)]);
        // A quiet frame notes it, and copies nothing.
        let out = frame(vec![]);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(selected(&ctx), text);
        // Right-click on it: the selection holds, and the menu opens.
        let m = rect.center();
        frame(vec![button(m, PointerButton::Secondary, true)]);
        frame(vec![button(m, PointerButton::Secondary, false)]);
        frame(vec![]);
        assert!(egui::Popup::is_any_open(&ctx), "Copy menu open");
        assert_eq!(selected(&ctx), text);
        assert!(ctx.plugin::<LabelSelectionState>().lock().has_selection());
        // Esc shuts the menu and lets the selection go.
        let esc = Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(vec![esc]);
        frame(vec![]);
        frame(vec![]);
        assert!(!egui::Popup::is_any_open(&ctx));
    }

    /// With text selected in a side panel, a canvas beside it (as the Map
    /// is) still pans on a drag, and a right-click on it opens its own
    /// menu, not the label's.
    #[test]
    fn a_selection_leaves_a_canvas_its_drags_and_its_menu() {
        let ctx = egui::Context::default();
        install(&ctx);
        let panned = std::cell::Cell::new(0.0_f32);
        let canvas_menu = std::cell::Cell::new(false);
        let rects = std::cell::Cell::new((egui::Rect::NOTHING, egui::Rect::NOTHING));
        let frame = |events: Vec<Event>| {
            let input = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let label = ui.label("Switch 198.51.100.4").rect;
                let area =
                    egui::Rect::from_min_size(egui::pos2(300.0, 100.0), egui::vec2(300.0, 300.0));
                let canvas = ui.interact(area, Id::new("canvas"), egui::Sense::click_and_drag());
                panned.set(panned.get() + canvas.drag_delta().x);
                canvas.context_menu(|ui| {
                    canvas_menu.set(true);
                    ui.label("Card menu");
                });
                rects.set((label, area));
                label_menu(ui.ctx());
            });
            out.textures_delta.clear();
        };
        let button = |pos, button, pressed| Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![]);
        let (label, area) = rects.get();
        let (a, b) = (
            label.left_center() + egui::vec2(1.0, 0.0),
            label.right_center(),
        );
        frame(vec![
            Event::PointerMoved(a),
            button(a, PointerButton::Primary, true),
        ]);
        frame(vec![Event::PointerMoved(b)]);
        frame(vec![button(b, PointerButton::Primary, false)]);
        frame(vec![]);
        assert!(!selected(&ctx).is_empty());
        // A drag on the canvas pans it.
        let c = area.center();
        frame(vec![
            Event::PointerMoved(c),
            button(c, PointerButton::Primary, true),
        ]);
        for dx in 1..=6 {
            frame(vec![Event::PointerMoved(
                c + egui::vec2(10.0 * dx as f32, 0.0),
            )]);
        }
        frame(vec![button(
            c + egui::vec2(60.0, 0.0),
            PointerButton::Primary,
            false,
        )]);
        assert!(panned.get() > 30.0, "panned {}", panned.get());
        // Select again, then right-click the canvas: its own menu.
        frame(vec![
            Event::PointerMoved(a),
            button(a, PointerButton::Primary, true),
        ]);
        frame(vec![Event::PointerMoved(b)]);
        frame(vec![button(b, PointerButton::Primary, false)]);
        frame(vec![]);
        frame(vec![
            Event::PointerMoved(c),
            button(c, PointerButton::Secondary, true),
        ]);
        frame(vec![button(c, PointerButton::Secondary, false)]);
        frame(vec![]);
        frame(vec![]);
        assert!(canvas_menu.get(), "the canvas's menu");
        let ours = Id::new("clip label menu");
        assert!(!egui::Popup::is_id_open(&ctx, ours), "not the label menu");
    }
}
