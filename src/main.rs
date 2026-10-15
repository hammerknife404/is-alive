#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod job;
mod net;
mod tools;
mod widgets;

use eframe::egui;

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Ping,
    Ports,
    Trace,
    Sweep,
    Dns,
    Results,
}

const TABS: &[(Tab, &str)] = &[
    (Tab::Ping, "Ping"),
    (Tab::Ports, "Port Check"),
    (Tab::Trace, "Traceroute"),
    (Tab::Sweep, "Subnet Sweep"),
    (Tab::Dns, "DNS Lookup"),
    (Tab::Results, "Results"),
];

struct App {
    tab: Tab,
    ping: tools::ping::PingTool,
    ports: tools::ports::PortTool,
    trace: tools::trace::TraceTool,
    sweep: tools::sweep::SweepTool,
    dns: tools::dns::DnsTool,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ping.poll();
        self.ports.poll();
        self.trace.poll();
        self.sweep.poll();
        self.dns.poll();
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("is-alive");
                ui.separator();
                for (tab, name) in TABS {
                    ui.selectable_value(&mut self.tab, *tab, *name);
                }
            });
            ui.add_space(2.0);
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Ping => self.ping.ui(ui),
            Tab::Ports => self.ports.ui(ui),
            Tab::Trace => self.trace.ui(ui),
            Tab::Sweep => self.sweep.ui(ui),
            Tab::Dns => self.dns.ui(ui),
            Tab::Results => self.results_ui(ui),
        });
    }
}

impl App {
    /// Every tool's current output, in tab order; tools that haven't run are skipped.
    fn outputs(&self) -> Vec<(&'static str, String)> {
        [
            ("Ping", self.ping.output()),
            ("Port Check", self.ports.output()),
            ("Traceroute", self.trace.output()),
            ("Subnet Sweep", self.sweep.output()),
            ("DNS Lookup", self.dns.output()),
        ]
        .into_iter()
        .filter_map(|(name, out)| Some((name, out?)))
        .collect()
    }

    fn results_ui(&mut self, ui: &mut egui::Ui) {
        let outputs = self.outputs();
        let all = || {
            outputs
                .iter()
                .map(|(name, text)| format!("=== {name} ===\n{}\n", text.trim_end()))
                .collect::<Vec<_>>()
                .join("\n")
        };
        ui.horizontal(|ui| {
            ui.label(format!("{} tool(s) with results", outputs.len()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!outputs.is_empty(), |ui| {
                    widgets::save_button(ui, "results", all);
                    widgets::copy_button(ui, all);
                });
            });
        });
        ui.separator();
        if outputs.is_empty() {
            ui.weak("No results yet. Run something in another tab and it shows up here.");
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (name, text) in &outputs {
                    egui::CollapsingHeader::new(egui::RichText::new(*name).strong())
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let stem = name.to_lowercase().replace(' ', "-");
                                widgets::save_button(ui, &stem, || text.clone());
                                widgets::copy_button(ui, || text.clone());
                            });
                            ui.add(
                                egui::Label::new(egui::RichText::new(text.trim_end()).monospace())
                                    .selectable(true),
                            );
                        });
                }
            });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("is-alive — network tester")
            .with_app_id("is-alive")
            .with_inner_size([860.0, 600.0])
            .with_min_inner_size([620.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "is-alive",
        options,
        Box::new(|_cc| {
            Ok(Box::new(App {
                tab: Tab::Ping,
                ping: Default::default(),
                ports: Default::default(),
                trace: Default::default(),
                sweep: Default::default(),
                dns: Default::default(),
            }))
        }),
    )
}
