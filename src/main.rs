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
}

const TABS: &[(Tab, &str)] = &[
    (Tab::Ping, "Ping"),
    (Tab::Ports, "Port Check"),
    (Tab::Trace, "Traceroute"),
    (Tab::Sweep, "Subnet Sweep"),
    (Tab::Dns, "DNS Lookup"),
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
        // Jobs on hidden tabs keep running; their results queue until shown.
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Ping => self.ping.ui(ui),
            Tab::Ports => self.ports.ui(ui),
            Tab::Trace => self.trace.ui(ui),
            Tab::Sweep => self.sweep.ui(ui),
            Tab::Dns => self.dns.ui(ui),
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("is-alive — network tester")
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
