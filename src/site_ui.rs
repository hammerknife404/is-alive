//! The site switcher: the chip beside the app's name, its menu (the sites,
//! Site Bundle, new, rename, delete), the dialogs it opens, and the switch itself.
//! See site.rs for what a site keeps.

use crate::{App, VERSION, site, theme, widgets};
use eframe::egui::{self, RichText};
use std::path::PathBuf;

/// What the site dialog is asking.
pub(crate) enum SiteAsk {
    /// Open the site in `to` (None: Scratch), called `name`.
    Switch {
        to: Option<String>,
        name: String,
    },
    New {
        name: String,
        carry: bool,
        err: Option<String>,
    },
    Rename {
        name: String,
        err: Option<String>,
    },
    Delete {
        folder: String,
        name: String,
    },
}

impl App {
    /// The open site's chip, beside the app's name: click for the sites.
    pub(crate) fn site_chip(&mut self, ui: &mut egui::Ui) {
        let scratch = site::current().is_none();
        let accent = theme::accent();
        let (fill, stroke, dot) = if scratch {
            let v = ui.visuals();
            (
                v.widgets.inactive.weak_bg_fill,
                v.widgets.inactive.bg_stroke.color,
                v.weak_text_color(),
            )
        } else {
            (
                accent.gamma_multiply(0.18),
                accent.gamma_multiply(0.7),
                widgets::ok(),
            )
        };
        let mut job = egui::text::LayoutJob::default();
        let style = ui.style().clone();
        RichText::new("● ").color(dot).append_to(
            &mut job,
            &style,
            egui::FontSelection::Default,
            egui::Align::Center,
        );
        RichText::new(format!("{} ▾", self.site_name))
            .color(ui.visuals().strong_text_color())
            .append_to(
                &mut job,
                &style,
                egui::FontSelection::Default,
                egui::Align::Center,
            );
        let chip = ui
            .add(
                egui::Button::new(job)
                    .fill(fill)
                    .stroke(egui::Stroke::new(1.0, stroke))
                    .corner_radius(10),
            )
            .on_hover_text(if scratch {
                "No site open: Scratch. Click to open a site or make one, so this visit's notes, maps and configs stay together."
            } else {
                "The site open now: Configs, Map, Notes, Results and the Site Bundle work on it. Click to switch or make a site."
            });
        if chip.clicked() {
            self.sites = site::list();
        }
        let mut ask = None;
        let mut bundle = false;
        let current = site::current();
        egui::Popup::menu(&chip)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(380.0);
                ui.weak(RichText::new("SITES").small());
                for s in &self.sites {
                    let open = s.folder == current;
                    let mut parts = Vec::new();
                    if open {
                        parts.push("Open now".to_string());
                    } else if s.folder.is_none() {
                        parts.push("No site".to_string());
                    } else {
                        parts.push(site::when(&s.opened));
                    }
                    if s.configs > 0 {
                        parts.push(format!("{} config{}", s.configs, plural(s.configs)));
                    }
                    if s.maps > 0 {
                        parts.push(format!("{} saved map{}", s.maps, plural(s.maps)));
                    }
                    if s.words > 0 {
                        parts.push(format!("notes {} word{}", s.words, plural(s.words)));
                    }
                    parts.retain(|p| !p.is_empty());
                    let mut job = egui::text::LayoutJob::default();
                    let style = ui.style().clone();
                    RichText::new(&s.name)
                        .color(ui.visuals().strong_text_color())
                        .append_to(
                            &mut job,
                            &style,
                            egui::FontSelection::Default,
                            egui::Align::Center,
                        );
                    RichText::new(format!("\n{}", parts.join(" · ")))
                        .small()
                        .color(ui.visuals().weak_text_color())
                        .append_to(
                            &mut job,
                            &style,
                            egui::FontSelection::Default,
                            egui::Align::Center,
                        );
                    if ui
                        .add(egui::Button::selectable(open, job))
                        .on_hover_text(format!("Kept in {}", s.path.display()))
                        .clicked()
                    {
                        if !open {
                            ask = Some(SiteAsk::Switch {
                                to: s.folder.clone(),
                                name: s.name.clone(),
                            });
                        }
                        ui.close();
                    }
                }
                ui.separator();
                if ui
                    .button("Site Bundle…")
                    .on_hover_text("Zip this site for /unifidump and /mapnetwork")
                    .clicked()
                {
                    bundle = true;
                    ui.close();
                }
                ui.separator();
                if ui.button("New site…").clicked() {
                    ask = Some(SiteAsk::New {
                        name: String::new(),
                        carry: false,
                        err: None,
                    });
                    ui.close();
                }
                if ui
                    .add_enabled(current.is_some(), egui::Button::new("Rename this site…"))
                    .on_disabled_hover_text("Scratch isn't a site")
                    .clicked()
                {
                    ask = Some(SiteAsk::Rename {
                        name: self.site_name.clone(),
                        err: None,
                    });
                    ui.close();
                }
                let others: Vec<&site::Info> = self
                    .sites
                    .iter()
                    .filter(|s| s.folder.is_some() && s.folder != current)
                    .collect();
                ui.add_enabled_ui(!others.is_empty(), |ui| {
                    ui.menu_button("Delete a site…", |ui| {
                        for s in others {
                            if ui.button(&s.name).clicked() {
                                ask = Some(SiteAsk::Delete {
                                    folder: s.folder.clone().unwrap_or_default(),
                                    name: s.name.clone(),
                                });
                                ui.close();
                            }
                        }
                    });
                });
            });
        if ask.is_some() {
            self.site_ask = ask;
        }
        if bundle {
            self.bundle.open = true;
        }
    }

    /// The dialog the switcher opened, if any.
    pub(crate) fn site_dialog(&mut self, ctx: &egui::Context) {
        if self.site_ask.is_none() {
            return;
        }
        let busy = self.switch_busy();
        // Autosave is off and the notes changed: a switch would drop them.
        let unsaved_notes = self.notes.unsaved();
        let unsaved = self.map.unsaved();
        let here = self.site_name.clone();
        let remembered = self.site_configs.len();
        let scratch = site::current().is_none();
        let keep = self.look.results_keep;
        let Some(ask) = &mut self.site_ask else {
            return;
        };
        let mut done = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("site_dialog")).show(ctx, |ui| {
            ui.set_max_width(480.0);
            match ask {
                SiteAsk::Switch { to, name } => {
                    ui.heading(format!("Open {name}?"));
                    ui.add_space(4.0);
                    ui.columns(2, |c| {
                        c[0].strong(format!("{here} keeps"));
                        c[0].label("• Its notes");
                        c[0].label("• Its saved maps and card notes");
                        if !scratch && keep {
                            c[0].label("• Its Results");
                        }
                        if !scratch {
                            c[0].label(match remembered {
                                0 => "• No configs to remember".to_string(),
                                1 => "• 1 config, by where it is".to_string(),
                                n => format!("• {n} configs, by where they are"),
                            });
                        }
                        c[1].strong("Then cleared from memory");
                        c[1].label("• The configs, and their passwords and keys");
                        c[1].label("• The sweep and the live map");
                        c[1].label("• Results and the Site Bundle's details");
                    });
                    ui.add_space(4.0);
                    ui.weak(if to.is_some() {
                        let results = if keep { ", its Results" } else { "" };
                        format!("{name} opens as it was left: its notes and maps{results}, its configs read again from where they are.")
                    } else {
                        "Scratch is no site: the notes and maps kept outside any site.".to_string()
                    });
                    if !unsaved.is_empty() {
                        ui.colored_label(
                            widgets::warn(),
                            format!(
                                "Saved map{} with changes not saved: {}. Update {} first, or the changes are lost.",
                                plural(unsaved.len()),
                                unsaved.join(", "),
                                if unsaved.len() == 1 { "it" } else { "them" }
                            ),
                        );
                    }
                    if unsaved_notes {
                        ui.colored_label(
                            widgets::warn(),
                            format!("The notes have changes not saved (Autosave is off). Save them in {here} first?"),
                        );
                    }
                    if let Some(why) = busy {
                        ui.colored_label(widgets::warn(), why);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if unsaved_notes {
                            if ui
                                .add_enabled(busy.is_none(), egui::Button::new("Save notes and switch"))
                                .clicked()
                            {
                                done = Some((to.clone(), false, true));
                            }
                            if ui
                                .add_enabled(
                                    busy.is_none(),
                                    egui::Button::new(RichText::new("Switch, don't save").color(widgets::bad())),
                                )
                                .clicked()
                            {
                                done = Some((to.clone(), false, false));
                            }
                        } else if ui.add_enabled(busy.is_none(), egui::Button::new("Switch")).clicked() {
                            done = Some((to.clone(), false, false));
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                SiteAsk::New { name, carry, err } => {
                    ui.heading("New site");
                    ui.label("Name");
                    let r = widgets::line(
                        ui,
                        crate::input::Field::Text,
                        name,
                        440.0,
                        "Client and building, e.g. Harbor Dental front office",
                    );
                    // Enter while the name has (or just had) the keyboard.
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && (r.has_focus() || r.lost_focus());
                    // The name has the keyboard when the dialog opens.
                    if !enter && ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.add_space(4.0);
                    ui.label("Start with");
                    ui.radio_value(carry, false, "An empty site");
                    ui.radio_value(carry, true, format!("What's open now ({here}: the notes, configs and map as they are)"));
                    ui.weak("Kept in your profile folder under sites: the notes (redacted), saved maps and card notes. Configs stay where they are; the site remembers their paths.");
                    widgets::error(ui, err);
                    if !*carry && let Some(why) = busy {
                        ui.colored_label(widgets::warn(), format!("{why} Or start with what's open."));
                    }
                    if !*carry && unsaved_notes {
                        ui.weak(format!("The notes' changes not saved (Autosave is off) are saved in {here} first."));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let ok = !name.trim().is_empty() && (*carry || busy.is_none());
                        if ui.add_enabled(ok, egui::Button::new("Create and open")).clicked() || (ok && enter) {
                            match site::create(name) {
                                Ok(folder) => done = Some((Some(folder), *carry, !*carry)),
                                Err(e) => *err = Some(e),
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                SiteAsk::Rename { name, err } => {
                    ui.heading("Rename this site");
                    let r = widgets::line(ui, crate::input::Field::Text, name, 440.0, "Site name");
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && (r.has_focus() || r.lost_focus());
                    if !enter && ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    widgets::error(ui, err);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Rename").clicked() || enter {
                            match site::current().map(|f| site::rename(&f, name)) {
                                Some(Ok(())) => {
                                    self_rename(ctx);
                                    close = true;
                                }
                                Some(Err(e)) => *err = Some(e),
                                None => close = true,
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                SiteAsk::Delete { folder, name } => {
                    ui.heading(format!("Delete {name}?"));
                    ui.label("Its notes, saved maps and card notes are deleted from this computer. The configs it opened stay where they are.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Delete").color(widgets::bad()))
                            .clicked()
                        {
                            match site::delete(folder) {
                                Ok(()) => widgets::toast(format!("{name} deleted"), true),
                                Err(e) => widgets::toast(format!("Couldn't delete {name}: {e}"), false),
                            }
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if close || modal.should_close() {
            self.site_ask = None;
        }
        if renamed(ctx) {
            self.site_name = site::name();
            self.retitle(ctx);
        }
        if let Some((to, carry, save_notes)) = done {
            self.site_ask = None;
            self.switch_site(ctx, to, carry, save_notes);
        }
    }

    /// Save the open site, clear what's in memory, and open `to` (None:
    /// Scratch). With `carry`, a new site takes what's open along instead.
    /// `save_notes`: write the notes first even with Autosave off (the user
    /// chose to); otherwise they're saved only if Autosave is on.
    pub(crate) fn switch_site(
        &mut self,
        ctx: &egui::Context,
        to: Option<String>,
        carry: bool,
        save_notes: bool,
    ) {
        let now = ctx.input(|i| i.time);
        // What's open is saved where it belongs first.
        self.notes.set_elsewhere(self.map.note_texts());
        if save_notes {
            self.notes.save_now();
        } else {
            self.notes.flush();
        }
        self.remember_site_configs();
        self.keep_results();
        // A reload asked for here isn't the next site's.
        self.reloading = false;
        site::set_current(to);
        site::touch();
        self.look.site = site::current();
        theme::save(&self.look);
        self.site_name = site::name();
        if carry {
            // The notes, configs, sweep and live map are this site's now.
            self.notes.rehome(now);
            self.map.switch_site(true);
            self.site_configs.clear();
            // Results too: written to the new site's folder next frame.
            self.log_saved = (usize::MAX, usize::MAX, usize::MAX);
        } else {
            self.export.clear();
            self.sweep.clear_results();
            self.load_results();
            // The next site's notes, in the view they were read in.
            let fresh = crate::tools::notes::NotesTool::default();
            let old = std::mem::replace(&mut self.notes, fresh);
            self.notes.keep_view_of(&old);
            // A fresh Notes starts with Autosave on: keep the user's choice.
            self.notes.set_autosave(self.look.notes_autosave);
            self.map.switch_site(false);
            self.site_missing.clear();
            self.site_configs = site::meta().map(|m| m.configs).unwrap_or_default();
            self.reopen_site_configs(ctx);
        }
        let named = site::current().is_some().then(|| self.site_name.clone());
        self.bundle.for_site(named.as_deref());
        self.report.for_site(named.as_deref());
        self.retitle(ctx);
        widgets::toast(format!("{} is open", self.site_name), true);
    }

    /// Why a site can't be switched now, if it can't: a switch would cut
    /// short a run, a config being read (dropping it from the site's list)
    /// or a Site Bundle being written.
    pub(crate) fn switch_busy(&self) -> Option<&'static str> {
        if self.any_running() {
            Some("Stop the running tools first.")
        } else if self.export.is_running() {
            Some("Wait for Config Export to finish reading.")
        } else if self.export.picking() {
            // What's picked would open in the next site.
            Some("Pick the files to open, or cancel, first.")
        } else if self.bundle.is_running() {
            Some("Wait for the Site Bundle to be saved.")
        } else {
            None
        }
    }

    /// Read the open site's configs again from where they are; those not
    /// found stay on its list until Clear map.
    pub(crate) fn reopen_site_configs(&mut self, ctx: &egui::Context) {
        let Some(m) = site::meta() else { return };
        let (here, gone): (Vec<PathBuf>, Vec<PathBuf>) =
            m.configs.into_iter().partition(|p| p.is_file());
        self.export.reopen(ctx, here);
        if !gone.is_empty() {
            let names: Vec<String> = gone
                .iter()
                .map(|p| crate::tools::export::file_name(p))
                .collect();
            widgets::toast(
                format!(
                    "{} config{} not where {} was: {}. Plug in the drive and switch again, or Clear map to forget {}.",
                    gone.len(),
                    plural(gone.len()),
                    if gone.len() == 1 { "it" } else { "they were" },
                    names.join(", "),
                    if gone.len() == 1 { "it" } else { "them" }
                ),
                false,
            );
        }
        self.site_missing = gone;
    }

    /// Once a frame: the site remembers the configs open now (and those it
    /// couldn't find), once Config Export has finished reading.
    pub(crate) fn remember_site_configs(&mut self) {
        if self.export.is_running() || site::current().is_none() {
            return;
        }
        // Those it couldn't read stay too: they're tried again next time.
        let mut open = self.export.remembered_paths();
        for p in &self.site_missing {
            if !open.contains(p) {
                open.push(p.clone());
            }
        }
        if open != self.site_configs {
            site::remember_configs(&open);
            self.site_configs = open;
        }
    }

    /// The window's title names the open site.
    pub(crate) fn retitle(&self, ctx: &egui::Context) {
        let title = match site::current() {
            Some(_) => format!("is-alive {VERSION} — {}", self.site_name),
            None => format!("is-alive {VERSION} — network tester"),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// A rename landed: picked up after the dialog (it can't reach the App).
fn self_rename(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("site_renamed"), true));
}

fn renamed(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new("site_renamed")))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn plurals() {
        assert_eq!(super::plural(1), "");
        assert_eq!(super::plural(2), "s");
    }
}
