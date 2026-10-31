use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::hostinfo::HostInfo;
use crate::net::icmp;
use crate::net::mac::{self, Mac};
use crate::net::{self, TcpState, ports, targets};
use crate::widgets::{self, Action};
use eframe::egui::{self, RichText, Ui};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Host {
    ip: Ipv4Addr,
    /// None for a host that never answered ping but did answer ARP.
    rtt: Option<f64>,
    ttl: Option<u8>,
    name: Option<String>,
    mac: Option<Mac>,
    /// Open ports from `ports::INFRA`, ascending.
    open: Vec<u16>,
}

enum Ev {
    Probed,
    Alive(Host),
    Mac(Ipv4Addr, Mac),
    /// Port phase starting: this many more probes.
    PortProbes(usize),
    Open(Ipv4Addr, u16),
}

pub struct SweepTool {
    range: String,
    detected: bool,
    timeout_ms: u32,
    threads: usize,
    rate: u32,
    resolve_names: bool,
    check_ports: bool,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    started_at: Instant,
    total: usize,
    probed: usize,
    alive: Vec<Host>,
    /// This machine and its gateway when the run started, to mark them.
    local: HostInfo,
    /// Whether the last run checked management ports.
    checked_ports: bool,
    err: Option<String>,
    /// Bumped whenever the host list changes, so the map knows to rebuild.
    revision: u64,
}

impl Default for SweepTool {
    fn default() -> Self {
        Self {
            range: "192.168.1.0/24".into(),
            detected: false,
            timeout_ms: 750,
            threads: widgets::SPEEDS[widgets::DEFAULT_SPEED].1,
            rate: widgets::SPEEDS[widgets::DEFAULT_SPEED].2,
            resolve_names: true,
            check_ports: false,
            job: None,
            run_started: None,
            started_at: Instant::now(),
            total: 0,
            probed: 0,
            alive: Vec::new(),
            local: HostInfo::default(),
            checked_ports: false,
            err: None,
            revision: 0,
        }
    }
}

/// How often the ping phase rereads the confirmed ARP entries. Linux keeps
/// an entry REACHABLE for 15–45 s, so this catches every one the sweep's
/// own ARP requests produce.
const ARP_EVERY: Duration = Duration::from_secs(3);

const ARP_TIP: &str = "Answered this machine's ARP request but not ping (a firewall or an \
endpoint that drops ICMP). Only entries the OS has just confirmed count, not stale ones.";

const PORTS_TIP: &str = "After the sweep, try a TCP connect to each live host's management ports \
(SSH, telnet, web, UniFi 8080/8443, FortiGate 541, SonicWall 4433, MikroTik 8291) and a few that \
mark a role (SMB, RDP, printers 9100, cameras 554). Evidence for telling switches, APs and \
firewalls from clients. Same speed limit as the sweep, but it is a port scan: expect IDS/EDR alerts.";

impl SweepTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        let mut got = false;
        let was = self.job.is_some();
        let mut changed = false;
        let running = pump(&mut self.job, |ev| match ev {
            Ev::Probed => self.probed += 1,
            Ev::Alive(h) => {
                self.alive.push(h);
                got = true;
            }
            Ev::Mac(ip, m) => {
                if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                    h.mac = Some(m);
                    changed = true;
                }
            }
            Ev::PortProbes(n) => self.total += n,
            Ev::Open(ip, port) => {
                if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                    h.open.push(port);
                    h.open.sort_unstable();
                    changed = true;
                }
            }
        });
        if got {
            self.alive.sort_by_key(|h| h.ip);
        }
        if got || changed || was != running {
            self.revision += 1;
        }
        running
    }

    pub fn ui(&mut self, ui: &mut Ui, info: &HostInfo) {
        // First time the tab is shown: prefill with the host's own subnet.
        if !self.detected {
            self.detected = true;
            if let Some(r) = targets::local_subnet() {
                self.range = r;
            }
        }
        let running = self.poll();

        let mut enter = false;
        ui.horizontal(|ui| {
            enter |= widgets::field(
                ui,
                "Range",
                &mut self.range,
                220.0,
                "10.0.0.0/24 or 10.0.0.1-254",
            );
            ui.separator();
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => self.job.as_ref().map_or((), Job::stop),
                Action::None => {}
            }
        });
        ui.horizontal(|ui| {
            widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=10_000, " ms");
            widgets::speed(ui, "sweep_speed", &mut self.threads, &mut self.rate);
            ui.checkbox(&mut self.resolve_names, "Resolve names");
            ui.checkbox(&mut self.check_ports, "Check mgmt ports")
                .on_hover_text(PORTS_TIP);
        });
        widgets::error(ui, &self.err);
        ui.separator();

        ui.horizontal(|ui| {
            if self.total > 0 {
                ui.label(
                    RichText::new(format!("{} alive", self.alive.len()))
                        .color(widgets::ok())
                        .strong(),
                );
                let arp = self.arp_only();
                if arp > 0 {
                    ui.label(RichText::new(format!("({arp} ARP only)")).weak())
                        .on_hover_text(ARP_TIP);
                }
                ui.label(format!("· {} / {} probed", self.probed, self.total));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(self.total > 0 && !running, |ui| {
                    if ui
                        .button("Inventory…")
                        .on_hover_text("Save live hosts as JSON for /mapnetwork")
                        .clicked()
                    {
                        widgets::save_file(
                            "is-alive-inventory.json".into(),
                            "JSON",
                            vec!["json"],
                            self.inventory(info),
                        );
                    }
                });
                ui.add_enabled_ui(self.total > 0, |ui| {
                    widgets::save_button(ui, "sweep", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
            });
        });
        if running && self.total > 0 {
            widgets::progress(ui, self.probed, self.total, self.started_at);
        }
        if self.alive.is_empty() && !running {
            let hint = if self.total > 0 {
                "No hosts answered ping or ARP. Past a router only ping counts, and firewalls often drop it; try Port Check on a known host."
            } else {
                "Enter a CIDR or range and press Enter."
            };
            widgets::empty_hint(ui, hint);
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Grid::new("sweep")
                    .striped(true)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        let mut heads = vec!["Address", "Time", "TTL", "MAC", "Vendor", "Name"];
                        if self.checked_ports {
                            heads.push("Open ports");
                        }
                        for h in heads {
                            ui.strong(h);
                        }
                        ui.end_row();
                        for h in &self.alive {
                            ui.monospace(h.ip.to_string());
                            match h.rtt {
                                Some(r) => ui.monospace(widgets::fmt_ms(r)),
                                None => ui
                                    .label(RichText::new("no ping").weak())
                                    .on_hover_text(ARP_TIP),
                            };
                            ui.monospace(h.ttl.map(|t| t.to_string()).unwrap_or_default());
                            ui.monospace(h.mac.as_ref().map(mac::fmt).unwrap_or_default());
                            ui.label(self.vendor(h));
                            ui.label(h.name.as_deref().unwrap_or(""));
                            if self.checked_ports {
                                ui.monospace(port_list(&h.open));
                            }
                            ui.end_row();
                        }
                    });
            });
    }

    fn arp_only(&self) -> usize {
        self.alive.iter().filter(|h| h.rtt.is_none()).count()
    }

    /// Vendor, or what the host is to this machine when it has no MAC here.
    fn vendor(&self, h: &Host) -> String {
        match &h.mac {
            Some(m) => mac::vendor_label(m),
            None if self.local.lan.is_some_and(|(ip, _)| ip == h.ip) => "(this machine)".into(),
            None => String::new(),
        }
    }

    /// Start time and logged text of a run that has just ended, once.
    pub fn take_finished(&mut self) -> Option<(String, String, Option<Table>)> {
        if self.job.is_some() {
            return None;
        }
        let started = self.run_started.take()?;
        let out = self.output();
        let table = out.as_ref().and_then(|_| self.table());
        Some((started, log::body(out, &self.err), table))
    }

    /// Start time of the run in progress.
    pub fn running_since(&self) -> Option<&str> {
        self.job.as_ref().and(self.run_started.as_deref())
    }

    /// Report text for the Results tab; None until something has run.
    pub fn output(&self) -> Option<String> {
        (self.total > 0).then(|| self.report())
    }

    fn report(&self) -> String {
        let mut s = format!("Sweep of {}: {} alive", self.range.trim(), self.alive.len());
        match self.arp_only() {
            0 => s += "\n",
            n => s += &format!(" ({n} found by ARP only, no ping reply)\n"),
        }
        for h in &self.alive {
            let rtt = match h.rtt {
                Some(r) => format!("{r:.1} ms"),
                None => "ARP only".into(),
            };
            let mut line = format!(
                "{:<15}  {:>9}  {:<17}  {:<24}  {}",
                h.ip,
                rtt,
                h.mac.as_ref().map(mac::fmt).unwrap_or_default(),
                self.vendor(h),
                h.name.as_deref().unwrap_or("")
            );
            if !h.open.is_empty() {
                line += &format!("  [{}]", port_list(&h.open));
            }
            s += line.trim_end();
            s += "\n";
        }
        s
    }

    /// Alive hosts only; the dead ones are the gaps.
    fn table(&self) -> Option<Table> {
        let rows = self
            .alive
            .iter()
            .map(|h| {
                vec![
                    self.range.trim().to_string(),
                    h.ip.to_string(),
                    h.rtt.map(log::ms).unwrap_or_default(),
                    log::cell(h.ttl),
                    h.mac.as_ref().map(mac::fmt).unwrap_or_default(),
                    self.vendor(h),
                    h.name.clone().unwrap_or_default(),
                    port_list(&h.open),
                    found_by(h).into(),
                ]
            })
            .collect();
        Some(Table {
            columns: &[
                "range",
                "ip",
                "rtt_ms",
                "ttl",
                "mac",
                "vendor",
                "name",
                "open_ports",
                "found_by",
            ],
            rows,
        })
    }

    /// Site inventory for /mapnetwork: facts only, no role guesses.
    pub fn inventory(&self, info: &HostInfo) -> String {
        let hosts: Vec<_> = self
            .alive
            .iter()
            .map(|h| {
                json!({
                    "ip": h.ip.to_string(),
                    "mac": h.mac.as_ref().map(mac::fmt),
                    "vendor": h.mac.as_ref().and_then(mac::vendor),
                    "private_mac": h.mac.as_ref().map(mac::local_admin),
                    "name": h.name,
                    "found_by": found_by(h),
                    "rtt_ms": h.rtt.map(|r| (r * 100.0).round() / 100.0),
                    "ttl": h.ttl,
                    "open_ports": self.checked_ports.then(|| h.open.iter().map(|&p| json!({
                        "port": p,
                        "service": ports::service_name(p),
                    })).collect::<Vec<_>>()),
                    "is_gateway": self.local.gateway == Some(h.ip),
                    "is_this_machine": self.local.lan.is_some_and(|(ip, _)| ip == h.ip),
                })
            })
            .collect();
        let v = json!({
            "source": format!("is-alive {} subnet sweep", crate::VERSION),
            "scanned_at": self.run_started.clone().unwrap_or_else(log::now),
            "range": self.range.trim(),
            "scanner": {
                "lan": self.local.lan.or(info.lan).map(|(ip, bits)| format!("{ip}/{bits}")),
                "gateway": self.local.gateway.or(info.gateway).map(|g| g.to_string()),
                "wan": info.wan.map(|w| w.to_string()),
            },
            "ports_checked": if self.checked_ports { ports::INFRA } else { &[] },
            "notes": [
                "Hosts that answered ping (found_by ping) or, on the scanner's own segment, answered ARP but dropped ping (found_by arp, rtt_ms null). A host that does neither is missed, so absence is not proof.",
                "mac comes from this machine's ARP table, so only hosts on the scanner's own segment have one; past a router it is null.",
                "vendor is the IEEE registry owner of the MAC prefix (null if unregistered); private_mac true means a locally administered address (a phone or laptop's private per-network MAC, or a virtual NIC) with no vendor.",
                "open_ports is null when ports were not checked.",
            ],
            "hosts": hosts,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
    }

    /// Start a run with the given target (used by the full scan).
    pub fn run(&mut self, ctx: &egui::Context, range: &str) {
        self.range = range.into();
        self.detected = true;
        self.start(ctx);
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// While running: hosts probed, how many in all, since when, and the range.
    pub fn progress(&self) -> Option<(usize, usize, Instant, &str)> {
        self.job
            .is_some()
            .then(|| (self.probed, self.total, self.started_at, self.range.trim()))
    }

    /// Range and live-host count of a finished run, if there is one.
    pub fn finished(&self) -> Option<(String, usize)> {
        (self.total > 0 && self.job.is_none())
            .then(|| (self.range.trim().to_string(), self.alive.len()))
    }

    /// Whether the last run checked management ports.
    pub fn checked_ports(&self) -> bool {
        self.checked_ports
    }

    /// Hosts found so far, for the map.
    pub fn seen(&self) -> Vec<crate::map::Seen> {
        self.alive
            .iter()
            .map(|h| crate::map::Seen {
                ip: h.ip,
                mac: h.mac,
                name: h.name.clone(),
                rtt: h.rtt,
                open: h.open.clone(),
            })
            .collect()
    }

    /// Changes whenever `seen` would.
    pub fn generation(&self) -> u64 {
        self.revision
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.err = None;
        self.revision += 1;
        self.alive.clear();
        self.probed = 0;
        let ips = match targets::parse_range(&self.range) {
            Ok(v) => v,
            Err(e) => return self.err = Some(e),
        };
        self.total = ips.len();
        self.local = HostInfo::local();
        self.checked_ports = self.check_ports;
        let (timeout, threads, rate, names, check_ports) = (
            self.timeout_ms,
            self.threads,
            self.rate,
            self.resolve_names,
            self.check_ports,
        );

        self.run_started = Some(log::now());
        self.started_at = Instant::now();
        let me = self.local.lan.map(|(ip, _)| ip);
        self.job = Some(Job::spawn(ctx, move |tx| {
            let alive = Mutex::new(Vec::new());
            // Each probe makes the OS ARP for the address, so hosts that drop
            // ping still land in the neighbor table as confirmed entries.
            let arp: Mutex<HashMap<Ipv4Addr, Mac>> = Mutex::new(HashMap::new());
            let last_look = Mutex::new(Instant::now());
            let look = || arp.lock().unwrap().extend(mac::confirmed());
            net::par_for(&ips, threads, rate, tx.cancel_flag(), |&ip| {
                let r = icmp::echo(IpAddr::V4(ip), 128, timeout, 32);
                if r.is_reply() {
                    // The reply means the neighbor entry was just resolved.
                    let mac = mac::neighbors().get(&ip).copied();
                    let name = if names {
                        net::reverse(IpAddr::V4(ip))
                    } else {
                        None
                    };
                    alive.lock().unwrap().push((ip, mac.is_some()));
                    tx.send(Ev::Alive(Host {
                        ip,
                        rtt: Some(r.rtt_ms),
                        ttl: r.ttl,
                        name,
                        mac,
                        open: Vec::new(),
                    }));
                }
                tx.send(Ev::Probed);
                // One worker at a time; the rest don't wait for it.
                if let Ok(mut t) = last_look.try_lock()
                    && t.elapsed() >= ARP_EVERY
                {
                    *t = Instant::now();
                    look();
                }
            });
            if !tx.cancelled() {
                look();
            }
            let alive = alive.into_inner().unwrap();

            // Catch entries that weren't in the table at the first look.
            let table = mac::neighbors();
            for &(ip, _) in alive.iter().filter(|(_, had)| !had) {
                if let Some(m) = table.get(&ip) {
                    tx.send(Ev::Mac(ip, *m));
                }
            }

            if tx.cancelled() {
                return;
            }
            let mut seen: HashSet<Ipv4Addr> = alive.iter().map(|a| a.0).collect();
            let swept: HashSet<Ipv4Addr> = ips.iter().copied().collect();
            let silent: Vec<(Ipv4Addr, Mac)> = arp
                .into_inner()
                .unwrap()
                .into_iter()
                .filter(|(ip, _)| swept.contains(ip) && !seen.contains(ip) && Some(*ip) != me)
                .collect();
            net::par_for(&silent, threads, rate, tx.cancel_flag(), |&(ip, m)| {
                let name = if names {
                    net::reverse(IpAddr::V4(ip))
                } else {
                    None
                };
                tx.send(Ev::Alive(Host {
                    ip,
                    rtt: None,
                    ttl: None,
                    name,
                    mac: Some(m),
                    open: Vec::new(),
                }));
            });
            seen.extend(silent.iter().map(|s| s.0));

            if !check_ports || tx.cancelled() {
                return;
            }
            let mut probes: Vec<(Ipv4Addr, u16)> = Vec::new();
            for ip in seen {
                probes.extend(ports::INFRA.iter().map(|&p| (ip, p)));
            }
            tx.send(Ev::PortProbes(probes.len()));
            let wait = Duration::from_millis(timeout as u64);
            net::par_for(&probes, threads, rate, tx.cancel_flag(), |&(ip, port)| {
                let addr = SocketAddr::new(IpAddr::V4(ip), port);
                if net::tcp_check(addr, wait).0 == TcpState::Open {
                    tx.send(Ev::Open(ip, port));
                }
                tx.send(Ev::Probed);
            });
        }));
    }
}

fn found_by(h: &Host) -> &'static str {
    if h.rtt.is_some() { "ping" } else { "arp" }
}

fn port_list(open: &[u16]) -> String {
    open.iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
