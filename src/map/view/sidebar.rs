//! The controls drawn over the map: zoom buttons, the sidebar of sites,
//! roles and filters, and the legend.

use super::*;

impl MapView {
    /// − 100% + Fit in the canvas corner, for when there's no wheel.
    pub(super) fn zoom_controls(&mut self, ui: &mut Ui, rect: Rect) {
        let h = 22.0;
        let mut x = rect.max.x - 10.0;
        let y = rect.max.y - 10.0 - h;
        let mut button = |ui: &mut Ui, w: f32, text: &str, tip: &str| {
            x -= w;
            let r = Rect::from_min_size(pos2(x, y), vec2(w, h));
            x -= 4.0;
            ui.put(r, egui::Button::new(text).corner_radius(6))
                .on_hover_text(tip)
                .clicked()
        };
        let fit = button(ui, 36.0, "Fit", "Show the whole map");
        let center = button(
            ui,
            56.0,
            "Center",
            "Move the middle of the map to the middle of the view, at this zoom",
        );
        let zoom_in = button(ui, 24.0, "+", "Zoom in");
        let pct = format!("{:.0}%", self.zoom * 100.0);
        let reset = button(ui, 48.0, &pct, "Back to 100%");
        let zoom_out = button(ui, 24.0, "−", "Zoom out");
        let target = if zoom_in {
            Some(self.zoom * 1.25)
        } else if zoom_out {
            Some(self.zoom / 1.25)
        } else if reset {
            Some(1.0)
        } else {
            None
        };
        if fit {
            self.follow = true;
            self.fit(rect);
        } else if center {
            self.center(rect);
        } else if let Some(z) = target {
            // Zoom about the middle of the canvas.
            let z = z.clamp(MIN_ZOOM, MAX_ZOOM);
            let mid = self.to_world(rect, rect.center());
            self.aim = Some((rect.size() / 2.0 - mid.to_vec2() * z, z));
            self.follow = false;
        }
    }

    /// Select every card at `site` (its card, when collapsed), lighting
    /// the whole group's connections, and bring its box into view.
    pub(super) fn show_site(&mut self, view: Rect, site: &str) {
        let cards: Vec<usize> = (0..self.map.nodes.len())
            .filter(|&i| {
                let n = &self.map.nodes[i];
                n.site.as_deref() == Some(site) && n.zone != Zone::Internet
            })
            .collect();
        let Some((&first, rest)) = cards.split_first() else {
            return;
        };
        self.select(Some(first));
        self.also = rest.iter().map(|&i| self.map.nodes[i].id.clone()).collect();
        let area = match self.lay.boxes.iter().find(|b| b.tree && b.title == site) {
            Some(b) => b.rect,
            None => cards
                .iter()
                .filter_map(|&i| self.target(i))
                .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_size(p, CARD))),
        };
        self.fit_to(view, area);
        self.follow = false;
    }

    /// Sites, roles to fade in and out, and the action items, worst first:
    /// the sidebar /mapnetwork's maps have.
    pub(super) fn sidebar(
        &mut self,
        ui: &mut Ui,
        focus: &mut Option<Vec<usize>>,
        focus_site: &mut Option<String>,
    ) {
        let sites = self.site_counts();
        let mut flip_all: Option<bool> = None;
        let counts: Vec<(Zone, usize)> = Zone::ALL
            .iter()
            .map(|&z| (z, self.map.nodes.iter().filter(|n| n.zone == z).count()))
            .filter(|c| c.1 > 0 && c.0 != Zone::Internet)
            .collect();
        let mut findings: Vec<(Sev, String, Vec<usize>)> = self
            .map
            .findings
            .iter()
            .map(|f| (f.sev, f.text.clone(), f.nodes.clone()))
            .collect();
        findings.sort_by_key(|f| f.0);
        let (mut hide, mut flip) = (false, None);
        egui::Panel::left("map_side")
            .resizable(true)
            .default_size(230.0)
            .size_range(180.0..=380.0)
            .show_collapsible(ui, &mut self.side, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(format!("{} cards", self.map.nodes.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        hide = ui
                            .small_button("‹")
                            .on_hover_text("Hide this panel")
                            .clicked();
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("map_side_scroll")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if !sites.is_empty() {
                            egui::CollapsingHeader::new(
                                RichText::new(format!("SITES ({})", sites.len()))
                                    .small()
                                    .strong(),
                            )
                            .id_salt("map_side_sites")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let any_open = sites.iter().any(|s| !s.2);
                                    let any_closed = sites.iter().any(|s| s.2);
                                    if ui
                                        .add_enabled(
                                            any_open,
                                            egui::Button::new("Collapse all").small(),
                                        )
                                        .on_hover_text("Fold every site to one card")
                                        .clicked()
                                    {
                                        flip_all = Some(true);
                                    }
                                    if ui
                                        .add_enabled(
                                            any_closed,
                                            egui::Button::new("Expand all").small(),
                                        )
                                        .on_hover_text("Open every site again")
                                        .clicked()
                                    {
                                        flip_all = Some(false);
                                    }
                                });
                                for (site, n, closed) in &sites {
                                    let row = ui.horizontal(|ui| {
                                        let (d, _) = ui
                                            .allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                        let toggle = ui.interact(
                                            d.expand(2.0),
                                            ui.id().with(("site_flip", site)),
                                            Sense::click(),
                                        );
                                        let c = if toggle.hovered() {
                                            ui.visuals().strong_text_color()
                                        } else {
                                            ui.visuals().text_color()
                                        };
                                        chevron(ui.painter(), d.center(), 4.5, !closed, c);
                                        let toggle = toggle.on_hover_text(if *closed {
                                            "Expand this site"
                                        } else {
                                            "Collapse this site to one card"
                                        });
                                        if toggle.clicked() {
                                            self.flip_site = Some(site.clone());
                                        }
                                        if toggle.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        let text = RichText::new(site);
                                        let text = if *closed { text.italics() } else { text };
                                        // Long names cut short, the count kept.
                                        let room = (ui.available_width() - 34.0).max(40.0);
                                        ui.allocate_ui(
                                            vec2(room, ui.spacing().interact_size.y),
                                            |ui| {
                                                ui.add(egui::Label::new(text).truncate())
                                                    .on_hover_text(site);
                                            },
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| ui.weak(n.to_string()),
                                        );
                                    });
                                    let r = ui
                                        .interact(
                                            row.response
                                                .rect
                                                .with_min_x(row.response.rect.min.x + 18.0),
                                            ui.id().with(("site_row", site)),
                                            Sense::click(),
                                        )
                                        .on_hover_text(
                                            "Show this site's cards and their connections",
                                        );
                                    if r.clicked() {
                                        *focus_site = Some(site.clone());
                                    }
                                    if r.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                }
                            });
                        }
                        egui::CollapsingHeader::new(RichText::new("ROLES").small().strong())
                            .id_salt("map_side_roles")
                            .default_open(true)
                            .show(ui, |ui| {
                                for (zone, n) in &counts {
                                    let off = self.hidden.contains(zone);
                                    let c = zone_colour(*zone);
                                    let r = ui
                                        .horizontal(|ui| {
                                            let (d, _) = ui.allocate_exact_size(
                                                vec2(12.0, 12.0),
                                                Sense::hover(),
                                            );
                                            if off {
                                                ui.painter().circle_stroke(
                                                    d.center(),
                                                    4.5,
                                                    Stroke::new(1.5, c),
                                                );
                                            } else {
                                                ui.painter().circle_filled(d.center(), 5.0, c);
                                            }
                                            let text = RichText::new(zone.name());
                                            ui.label(if off { text.weak() } else { text });
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| ui.weak(n.to_string()),
                                            );
                                        })
                                        .response
                                        .interact(Sense::click())
                                        .on_hover_text("Click to fade this role out or back in");
                                    if r.clicked() {
                                        flip = Some(*zone);
                                    }
                                    if r.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                }
                            });
                        if !findings.is_empty() {
                            egui::CollapsingHeader::new(
                                RichText::new(format!("ACTION ITEMS ({})", findings.len()))
                                    .small()
                                    .strong(),
                            )
                            .id_salt("map_side_actions")
                            .default_open(true)
                            .show(ui, |ui| {
                                for (sev, text, nodes) in &findings {
                                    let c = match sev {
                                        Sev::High => widgets::bad(),
                                        Sev::Med => widgets::warn(),
                                        Sev::Low => ui.visuals().weak_text_color(),
                                    };
                                    let r = ui
                                        .horizontal_top(|ui| {
                                            let (d, _) = ui.allocate_exact_size(
                                                vec2(10.0, 16.0),
                                                Sense::hover(),
                                            );
                                            ui.painter().circle_filled(
                                                d.center() - vec2(0.0, 2.0),
                                                4.0,
                                                c,
                                            );
                                            ui.add(egui::Label::new(text).wrap());
                                        })
                                        .response;
                                    if !nodes.is_empty() {
                                        let r = r
                                            .interact(Sense::click())
                                            .on_hover_text("Show on the map");
                                        if r.clicked() {
                                            *focus = Some(nodes.clone());
                                        }
                                        if r.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                    }
                                    ui.add_space(4.0);
                                }
                            });
                        }
                    });
            });
        if hide {
            self.side = false;
        }
        match flip_all {
            Some(true) => {
                self.closed = sites.into_iter().map(|s| s.0).collect();
                self.stale = true;
            }
            Some(false) => {
                self.closed.clear();
                self.stale = true;
            }
            None => {}
        }
        if let Some(z) = flip
            && !self.hidden.remove(&z)
        {
            self.hidden.insert(z);
        }
    }

    /// Role legend in the canvas corner; click a role to fade it out.
    pub(super) fn legend(&mut self, ui: &mut Ui, rect: Rect) {
        let mut counts: Vec<(Zone, usize)> = Zone::ALL
            .iter()
            .map(|&z| (z, self.map.nodes.iter().filter(|n| n.zone == z).count()))
            .filter(|c| c.1 > 0 && c.0 != Zone::Internet)
            .collect();
        if self.map.hidden > 0 {
            counts.retain(|c| c.1 > 0);
        }
        let font = FontId::proportional(11.5);
        let mut x = rect.min.x + 10.0;
        let y = rect.max.y - 10.0;
        let bg = ui.visuals().window_fill.gamma_multiply(0.92);
        for (zone, n) in counts {
            let label = format!("{}  {n}", zone.name());
            let g = ui
                .painter()
                .layout_no_wrap(label, font.clone(), ui.visuals().text_color());
            let r = Rect::from_min_size(
                pos2(x, y - g.size().y - 8.0),
                vec2(g.size().x + 26.0, g.size().y + 8.0),
            );
            // Room left for the zoom controls.
            if r.max.x > rect.max.x - 180.0 {
                break;
            }
            let resp = ui
                .interact(r, ui.id().with(("legend", zone)), Sense::click())
                .on_hover_text("Click to fade this role out or back in");
            if resp.clicked() && !self.hidden.remove(&zone) {
                self.hidden.insert(zone);
            }
            let off = self.hidden.contains(&zone);
            let p = ui.painter();
            p.rect(
                r,
                9,
                bg,
                Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                StrokeKind::Inside,
            );
            let c = zone_colour(zone);
            let dot = pos2(r.min.x + 11.0, r.center().y);
            if off {
                p.circle_stroke(dot, 4.0, Stroke::new(1.5, c));
            } else {
                p.circle_filled(dot, 4.5, c);
            }
            let tc = if off {
                ui.visuals().weak_text_color()
            } else {
                ui.visuals().text_color()
            };
            p.galley(pos2(r.min.x + 20.0, r.min.y + 4.0), g, tc);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            x = r.max.x + 6.0;
        }
    }
}
