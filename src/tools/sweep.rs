use crate::ais;
use crate::input::Field;
use crate::job::{Job, pump};
use crate::log::{self, Table};
use crate::net::hostinfo::HostInfo;
use crate::net::icmp;
use crate::net::mac::{self, Mac};
use crate::net::{self, TcpState, dnssd, names, ports, ssdp, subnet, targets, tls, wol};
use crate::theme;
use crate::unifi::Clients;
use crate::widgets::{self, Action};
use eframe::egui::{self, FontId, Rect, RichText, Sense, Ui, pos2, vec2};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Results (CSV export, the saved log): one row per result, these columns.
pub const COLUMNS: &[&str] = &[
    "range",
    "ip",
    "rtt_ms",
    "ttl",
    "mac",
    "vendor",
    "name",
    "open_ports",
    "found_by",
];

struct Host {
    ip: Ipv4Addr,
    /// None for a host that never answered ping but did answer ARP.
    rtt: Option<f64>,
    ttl: Option<u8>,
    name: Option<String>,
    /// Where the name came from: "dns" (its PTR), or the device's own word,
    /// "netbios" or "mdns".
    name_from: Option<&'static str>,
    mac: Option<Mac>,
    /// "arp" (this machine's neighbor table) or "netbios" (the device's own
    /// word, past a router).
    mac_from: Option<&'static str>,
    /// Open ports from `ports::INFRA`, ascending.
    open: Vec<u16>,
    /// All its management ports were tried. Smart skips phones and
    /// laptops; a host left out, or one a stopped run didn't finish, keeps
    /// everything the sweep found and says its ports weren't checked.
    checked: bool,
    /// Other MACs this address answered from during the sweep (the ARP
    /// table confirmed each): a duplicate IP, or a failover mid-sweep.
    also: Vec<Mac>,
    /// What its own web page calls it (its title, else its server), read
    /// after the port check where a web port answered.
    web: Option<String>,
    /// `web` is a real title, not a Server header standing in for a
    /// placeholder or missing one.
    titled: bool,
    /// The certificate on its HTTPS management port, read from the same
    /// handshake as that GET: the device's own claim, never trusted.
    cert: Option<Box<tls::Cert>>,
    /// What it said over DNS-SD (its service types, a printer's model):
    /// only devices on the scanner's own segment can answer.
    dnssd: Option<Box<dnssd::Found>>,
    /// What it said over SSDP (UPnP): same segment only.
    ssdp: Option<Box<ssdp::Found>>,
    /// What Advanced IP Scanner said about it, for a host from an
    /// imported scan.
    ais: Option<Box<ais::Row>>,
    /// Every way it answered the sweep.
    via: Via,
}

/// How a host was found: every way it answered. A host that answered
/// only DNS-SD or SSDP (silent to ping and ARP) is still listed, when
/// it's in the swept range and on the scanner's own segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Via {
    /// It answered ping.
    ping: bool,
    /// This machine's ARP table confirmed it during the sweep.
    arp: bool,
    /// It answered the DNS-SD browse (multicast DNS).
    mdns: bool,
    /// It answered the SSDP search.
    ssdp: bool,
    /// Advanced IP Scanner found it alive: a host from an imported scan.
    ais: bool,
}

impl Via {
    const PING: Via = Via {
        ping: true,
        arp: false,
        mdns: false,
        ssdp: false,
        ais: false,
    };
    const ARP: Via = Via {
        ping: false,
        arp: true,
        mdns: false,
        ssdp: false,
        ais: false,
    };
    const AIS: Via = Via {
        ping: false,
        arp: false,
        mdns: false,
        ssdp: false,
        ais: true,
    };

    /// As the inventory and CSV say it: "ping", "arp", "mdns", "ssdp",
    /// "ais".
    fn list(self) -> Vec<&'static str> {
        [
            (self.ping, "ping"),
            (self.arp, "arp"),
            (self.mdns, "mdns"),
            (self.ssdp, "ssdp"),
            (self.ais, "ais"),
        ]
        .into_iter()
        .filter_map(|(on, s)| on.then_some(s))
        .collect()
    }

    /// Found only by a multicast browse: no ping reply, no ARP entry.
    fn multicast_only(self) -> bool {
        !self.ping && !self.arp && (self.mdns || self.ssdp)
    }

    /// Which multicast it answered, for a label: "mDNS", "SSDP" or both.
    fn multicast(self) -> &'static str {
        match (self.mdns, self.ssdp) {
            (true, true) => "mDNS and SSDP",
            (true, false) => "mDNS",
            (false, true) => "SSDP",
            (false, false) => "",
        }
    }

    /// For the inspector: "ping, ARP, mDNS (DNS-SD)".
    fn words(self) -> String {
        self.list()
            .iter()
            .map(|v| match *v {
                "ping" => "ping",
                "arp" => "ARP",
                "mdns" => "mDNS (DNS-SD)",
                "ais" => "Advanced IP Scanner (imported)",
                _ => "SSDP (UPnP)",
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Ports a device's own web page may be on.
const WEB_PORTS: [u16; 6] = [80, 443, 4433, 5000, 8080, 8443];

/// The addresses that answered from more than one MAC, with every MAC
/// each answered from, in address order. `readings` is each confirmed
/// ARP entry seen during the sweep; only live hosts count.
fn conflicts(
    readings: &HashMap<Ipv4Addr, std::collections::BTreeSet<Mac>>,
    live: &HashSet<Ipv4Addr>,
) -> Vec<(Ipv4Addr, Vec<Mac>)> {
    let mut v: Vec<(Ipv4Addr, Vec<Mac>)> = readings
        .iter()
        .filter(|(ip, m)| m.len() > 1 && live.contains(ip))
        .map(|(ip, m)| (*ip, m.iter().copied().collect()))
        .collect();
    v.sort_unstable();
    v
}

/// Settings › Mgmt ports: which live hosts get their management ports
/// tried, after the sweep has found them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PortCheck {
    Off,
    /// The gateway, gear, NAS, printers, cameras and unknown makers; not
    /// phones and laptops (see `map::ports_worth_checking`).
    #[default]
    Smart,
    All,
}

impl PortCheck {
    const ALL: [PortCheck; 3] = [PortCheck::Off, PortCheck::Smart, PortCheck::All];

    fn name(self) -> &'static str {
        match self {
            PortCheck::Off => "Off",
            PortCheck::Smart => "Smart",
            PortCheck::All => "All hosts",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            PortCheck::Off => "No port checks: only what answers ping and ARP.",
            PortCheck::Smart => {
                "After the sweep, check the gateway, network gear, NAS, printers, cameras and \
                 devices from makers it doesn't know; skip what's clearly someone's phone or \
                 laptop (a private MAC, or a laptop or phone network card's maker such as Intel, \
                 Realtek, Apple or Samsung). Fewer probes for IDS/EDR to notice."
            }
            PortCheck::All => "After the sweep, check every live host, phones and laptops too.",
        }
    }
}

/// The vendor of another MAC an address answered from, nothing else
/// known about it.
fn map_vendor_label(m: &Mac) -> String {
    crate::map::vendor_label(crate::map::maker(m, false, false))
}

/// Of the live hosts (with the MAC each was found with), the ones whose
/// ports `mode` checks, in address order. Smart always checks the
/// gateway and never this machine.
fn port_targets(
    mode: PortCheck,
    hosts: &[(Ipv4Addr, Option<Mac>)],
    gateway: Option<Ipv4Addr>,
    me: Option<Ipv4Addr>,
) -> Vec<Ipv4Addr> {
    let mut v: Vec<Ipv4Addr> = hosts
        .iter()
        .filter(|(ip, m)| match mode {
            PortCheck::Off => false,
            PortCheck::All => true,
            PortCheck::Smart => {
                Some(*ip) == gateway
                    || (Some(*ip) != me
                        && crate::map::ports_worth_checking(
                            m.as_ref().and_then(mac::vendor),
                            m.as_ref().is_some_and(mac::local_admin),
                        ))
            }
        })
        .map(|h| h.0)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

enum Ev {
    Probed,
    Alive(Host),
    Mac(Ipv4Addr, Mac),
    /// Port phase starting on these hosts.
    PortTargets(Vec<Ipv4Addr>),
    /// Every management port of this host has been tried.
    PortDone(Ipv4Addr),
    /// This address answered from all these MACs.
    Conflict(Ipv4Addr, Vec<Mac>),
    /// This many more steps for the progress bar.
    More(usize),
    /// What this host's web page and its certificate say.
    Web(Ipv4Addr, Box<net::web::Named>),
    Open(Ipv4Addr, u16),
    /// The multicast discovery went out on the scanner's segment: the
    /// DNS-SD browse, the SSDP search.
    Browsed {
        dnssd: bool,
        ssdp: bool,
    },
    /// What these hosts (already found) said over DNS-SD.
    DnsSd(Vec<(Ipv4Addr, dnssd::Found)>),
    /// What these hosts (already found) said over SSDP.
    Ssdp(Vec<(Ipv4Addr, ssdp::Found)>),
}

/// What the box lists: (targets, addresses), or what's wrong with it.
type Plan = Result<(usize, usize), String>;

/// An Advanced IP Scanner export brought in as the last run: what it said
/// beyond its alive hosts.
#[derive(Clone, Debug, Default)]
struct Imported {
    /// The file's name, for the tab only: it may name the client, so it
    /// stays out of the report and the inventory.
    file: String,
    /// When Advanced IP Scanner ran, as it wrote it.
    date: Option<String>,
    dead: usize,
    unknown: usize,
    /// Rows with no address to read, or a second row for one address.
    skipped: usize,
    /// More rows than a sweep covers: the rest weren't read.
    cut: bool,
}

/// What reading a picked export gave: the file's name and the scan.
type Picked = Result<(String, ais::Scan), String>;

pub struct SweepTool {
    range: String,
    /// The range the last run swept, as typed then: what its report,
    /// table, inventory and handoff name, whatever the box says now.
    ran: String,
    /// `range` as last read for the box's count: (text, targets and addresses).
    plan: Option<(String, Plan)>,
    detected: bool,
    timeout_ms: u32,
    threads: usize,
    rate: u32,
    resolve_names: bool,
    ports: PortCheck,
    job: Option<Job<Ev>>,
    run_started: Option<String>,
    started_at: Instant,
    total: usize,
    probed: usize,
    alive: Vec<Host>,
    /// This machine and its gateway when the run started, to mark them.
    local: HostInfo,
    /// How the last run checked management ports.
    ran_ports: PortCheck,
    /// Hosts the port pass is on (shown as being checked until done).
    port_queue: HashSet<Ipv4Addr>,
    /// The last run was stopped before it finished: hosts it hadn't
    /// reached aren't listed, and ports it hadn't checked aren't "none".
    stopped: bool,
    err: Option<String>,
    /// Bumped whenever the host list changes, so the map knows to rebuild.
    revision: u64,
    /// How many sweeps have started.
    runs: u64,
    /// The host open in the inspector, by address.
    picked: Option<Ipv4Addr>,
    /// The last run sent a DNS-SD browse on the scanner's segment.
    browsed: bool,
    /// The last run sent an SSDP search on the scanner's segment.
    searched: bool,
    /// The last run was an imported Advanced IP Scanner export, not a sweep.
    imported: Option<Imported>,
    /// Import scan…: the file dialog, then the read, off the UI thread.
    importing: Option<Job<Picked>>,
}

impl Default for SweepTool {
    fn default() -> Self {
        Self {
            range: "192.168.1.0/24".into(),
            ran: String::new(),
            plan: None,
            detected: false,
            timeout_ms: 750,
            threads: widgets::SPEEDS[widgets::DEFAULT_SPEED].1,
            rate: widgets::SPEEDS[widgets::DEFAULT_SPEED].2,
            resolve_names: true,
            ports: PortCheck::default(),
            job: None,
            run_started: None,
            started_at: Instant::now(),
            total: 0,
            probed: 0,
            alive: Vec::new(),
            local: HostInfo::default(),
            ran_ports: PortCheck::Off,
            port_queue: HashSet::new(),
            stopped: false,
            err: None,
            revision: 0,
            runs: 0,
            picked: None,
            browsed: false,
            searched: false,
            imported: None,
            importing: None,
        }
    }
}

/// How often the ping phase rereads the confirmed ARP entries. Linux keeps
/// an entry REACHABLE for 15–45 s, so this catches every one the sweep's
/// own ARP requests produce.
const ARP_EVERY: Duration = Duration::from_secs(3);

const IMPORT_TIP: &str = "Bring in an Advanced IP Scanner scan: its XML export (File › Export \
› Export to XML), or rows pasted into a file. Its alive hosts take this tab's place as the last \
run, with names, MACs, makers, NetBIOS, OS, web titles and the services it found, and go to the \
Map, Results, the report and the Site Bundle like a sweep. Dead and unknown rows are counted, \
not imported. Logged-on users are never imported; shares and printers are counted, never named.";

const ARP_TIP: &str = "Answered this machine's ARP request but not ping (a firewall or an \
endpoint that drops ICMP). Only entries the OS has just confirmed count, not stale ones.";

const RANGE_TIP: &str = "One or more networks, separated by spaces, commas, semicolons or new \
lines:\n\n  10.10.10.0/24   a subnet (or 10.10.10.0 255.255.255.0, or 10.10.10.*)\n  \
192.168.100.1-50   a range in one subnet (or 192.168.100.1 - 192.168.100.50)\n  \
172.20.5.12   one address\n  10.0.0.0/28   a small block\n\nOverlaps are swept once; up to \
65,536 addresses in all. Past a router only ping answers (no ARP, no MACs).";

const PORTS_TIP: &str = "After the sweep, try a TCP connect to live hosts' management ports \
(SSH, telnet, web, UniFi 8080/8443, FortiGate 541, SonicWall 4433, MikroTik 8291), FTP (21, \
often left on in printers and NAS) and a few that mark a role (SMB, RDP, printers 9100, cameras \
554). A connect only: nothing is sent. Evidence for telling switches, APs and \
firewalls from clients. Same speed limit as the sweep, but it is a port scan: IDS/EDR may notice. \
Every host the sweep found stays listed, checked or not.";

impl SweepTool {
    /// Drain worker messages. Called every frame so results stay current
    /// while another tab is shown.
    pub fn poll(&mut self) -> bool {
        let mut picked = None;
        pump(&mut self.importing, |p| picked = Some(p));
        match picked {
            Some(Ok((file, scan))) => self.load_import(&file, scan),
            Some(Err(e)) => self.err = Some(e),
            None => {}
        }
        let was = self.job.is_some();
        let mut evs = Vec::new();
        let running = pump(&mut self.job, |ev| evs.push(ev));
        if self.apply(evs) || was != running {
            self.revision += 1;
        }
        running
    }

    /// Fold the workers' messages into the host list: hosts kept in address
    /// order, a MAC or open port joined to the host it belongs to. True
    /// when the hosts changed (the map then rebuilds).
    fn apply(&mut self, evs: Vec<Ev>) -> bool {
        let (mut got, mut changed) = (false, false);
        for ev in evs {
            match ev {
                Ev::Probed => self.probed += 1,
                Ev::Alive(h) => {
                    self.alive.push(h);
                    got = true;
                }
                Ev::Mac(ip, m) => {
                    if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                        h.mac = Some(m);
                        h.mac_from = Some("arp");
                        h.via.arp = true;
                        changed = true;
                    }
                }
                Ev::More(n) => self.total += n,
                Ev::Web(ip, named) => {
                    if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                        let net::web::Named {
                            label,
                            titled,
                            cert,
                        } = *named;
                        if label.is_some() {
                            (h.web, h.titled) = (label, titled);
                        }
                        h.cert = cert.map(Box::new).or(h.cert.take());
                        changed = true;
                    }
                }
                Ev::Conflict(ip, macs) => {
                    if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                        h.also = macs.into_iter().filter(|m| Some(*m) != h.mac).collect();
                        changed = true;
                    }
                }
                Ev::PortTargets(ips) => {
                    self.total += ips.len() * ports::INFRA.len();
                    self.port_queue.extend(ips);
                    changed = true;
                }
                Ev::PortDone(ip) => {
                    self.port_queue.remove(&ip);
                    if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                        h.checked = true;
                        changed = true;
                    }
                }
                Ev::Browsed { dnssd, ssdp } => {
                    self.browsed |= dnssd;
                    self.searched |= ssdp;
                }
                Ev::DnsSd(found) => {
                    for (ip, f) in found {
                        if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                            h.dnssd = Some(Box::new(f));
                            h.via.mdns = true;
                            changed = true;
                        }
                    }
                }
                Ev::Ssdp(found) => {
                    for (ip, f) in found {
                        if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                            h.ssdp = Some(Box::new(f));
                            h.via.ssdp = true;
                            changed = true;
                        }
                    }
                }
                Ev::Open(ip, port) => {
                    if let Some(h) = self.alive.iter_mut().find(|h| h.ip == ip) {
                        h.open.push(port);
                        h.open.sort_unstable();
                        changed = true;
                    }
                }
            }
        }
        if got {
            self.alive.sort_by_key(|h| h.ip);
        }
        got || changed
    }

    /// The tab; what the inspector asked for, if anything.
    pub fn ui(&mut self, ui: &mut Ui, info: &HostInfo) -> Option<crate::inspector::Ask> {
        let mut ask = None;
        // Docked, the inspector takes the tab's right side top to bottom;
        // floating, it's drawn over the table after it.
        let docked = matches!(
            crate::inspector::dock(ui.ctx().content_rect().width()),
            crate::inspector::Dock::Docked(_)
        );
        if docked {
            ask = self.inspector(ui);
        }
        // First time the tab is shown: prefill with the host's own subnet.
        if !self.detected {
            self.detected = true;
            if let Some(r) = targets::local_subnet() {
                self.range = r;
            }
        }
        let running = self.poll();

        let mut enter = false;
        // Wraps on a narrow window rather than cut off Start.
        ui.horizontal_wrapped(|ui| {
            widgets::lead(ui, "Ranges");
            // Wide enough for a few targets; it scrolls past that.
            let w = (ui.available_width() - 260.0).clamp(widgets::MAIN_W, 520.0);
            let r = widgets::line(
                ui,
                Field::Range,
                &mut self.range,
                w,
                &format!("e.g. {}", targets::EXAMPLES),
            )
            .on_hover_text(RANGE_TIP);
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            match widgets::run_button(ui, running, enter) {
                Action::Start => self.start(ui.ctx()),
                Action::Stop => {
                    if let Some(j) = &self.job {
                        j.stop();
                        self.stopped = true;
                    }
                }
                Action::None => {}
            }
            self.plan_label(ui);
        });
        let mut summary = vec![
            widgets::speed_name(self.threads, self.rate).to_string(),
            format!("timeout {} ms", self.timeout_ms),
            if self.resolve_names {
                "names"
            } else {
                "no names"
            }
            .to_string(),
        ];
        match self.ports {
            PortCheck::Off => {}
            PortCheck::Smart => summary.push("mgmt ports: smart".into()),
            PortCheck::All => summary.push("mgmt ports: all hosts".into()),
        }
        widgets::options(ui, "sweep", &summary, |ui| {
            ui.horizontal_wrapped(|ui| {
                widgets::num(ui, "Timeout", &mut self.timeout_ms, 100..=10_000, " ms");
                widgets::speed(ui, "sweep_speed", &mut self.threads, &mut self.rate);
                ui.checkbox(&mut self.resolve_names, "Resolve names")
                    .on_hover_text(
                        "Name each device: from DNS, else the name it gives itself over NetBIOS \
                     (Windows, NAS, many printers) or multicast DNS (Apple devices, printers, \
                     Linux). One small UDP question to each live device; off is quicker.\n\n\
                     On this computer's own network it also sends one DNS-SD (Bonjour) browse, \
                     which printers answer with their model, and one SSDP (UPnP) search. A \
                     device that answers only those, not ping or ARP, is listed too. Multicast \
                     stays on this segment: routed networks don't answer it",
                    );
                ui.label("Mgmt ports").on_hover_text(PORTS_TIP);
                for p in PortCheck::ALL {
                    ui.radio_value(&mut self.ports, p, p.name())
                        .on_hover_text(p.tip());
                }
            });
        });
        widgets::error(ui, &self.err);
        ui.separator();

        let mut import = false;
        widgets::status_row(
            ui,
            |ui| {
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
                    let mc = self.multicast_only();
                    if mc > 0 {
                        ui.label(RichText::new(format!("({mc} multicast only)")).weak())
                            .on_hover_text(MULTICAST_TIP);
                    }
                    match &self.imported {
                        Some(im) => {
                            // One count: a real scan has hundreds, and the
                            // buttons share this row at 1120 wide.
                            ui.label(format!("· imported · {} not alive", im.dead + im.unknown))
                                .on_hover_text(format!(
                                    "Imported from Advanced IP Scanner: {}{}. Only rows it \
                                     marked alive become hosts; {} dead and {} unknown are \
                                     counted, not imported.",
                                    im.file,
                                    im.date
                                        .as_ref()
                                        .map(|d| format!(", scanned {d}"))
                                        .unwrap_or_default(),
                                    im.dead,
                                    im.unknown
                                ));
                        }
                        None => {
                            ui.label(format!("· {} / {} probed", self.probed, self.total));
                        }
                    }
                }
            },
            |ui| {
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
                            // The tech's own copy, as the table shows it.
                            self.inventory(info, Clients::Full),
                        );
                    }
                });
                ui.add_enabled_ui(self.total > 0, |ui| {
                    widgets::save_button(ui, "sweep", || self.report());
                    widgets::copy_button(ui, || self.report());
                });
                ui.add_enabled_ui(!running && self.importing.is_none(), |ui| {
                    if ui
                        .button("Import scan…")
                        .on_hover_text(IMPORT_TIP)
                        .clicked()
                    {
                        import = true;
                    }
                });
            },
        );
        if import {
            self.import_dialog(ui.ctx());
        }
        if running && self.total > 0 {
            widgets::progress(ui, self.probed, self.total, self.started_at);
        }
        if self.alive.is_empty() && !running {
            if self.imported.is_some() {
                widgets::empty_state(
                    ui,
                    "No alive hosts in the scan",
                    "Advanced IP Scanner marked none of its rows alive; the dead and unknown ones aren't imported.",
                    &[],
                );
            } else if self.total > 0 {
                widgets::empty_state(
                    ui,
                    "No hosts answered",
                    "Nothing answered ping, ARP, DNS-SD or SSDP. Past a router only ping counts, and firewalls often drop it; try Port Check on a known host.",
                    &[],
                );
            } else if widgets::empty_state(
                ui,
                "Find what's on a network",
                    "Enter one or more subnets, ranges or addresses and press Enter. Every address is pinged and ARPed, with names, MACs and vendors. Scanned with Advanced IP Scanner? Import scan… brings in its XML export.",
                    &[("Import scan…", IMPORT_TIP, self.importing.is_none())],
                )
                .is_some()
                {
                    self.import_dialog(ui.ctx());
                }
            return ask;
        }

        // Floating, the inspector covers the table, not the run bar above it.
        let below = ui.available_rect_before_wrap();
        self.hosts_table(ui);
        if !docked {
            let mut over = ui.new_child(egui::UiBuilder::new().max_rect(below));
            ask = ask.or(self.inspector(&mut over));
        }
        ask
    }

    /// The hosts, one row each; a click opens one in the inspector (another
    /// click on it closes it). MAC and Vendor drop out, Vendor first, when the
    /// table gets too narrow: both are in the inspector.
    fn hosts_table(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        // An imported scan has no TTL: the column goes, so MAC fits.
        let cols = columns(width, self.checked_ports(), self.imported.is_none());
        let row_h = theme::TARGET_H;
        let font = FontId::proportional(crate::theme::BODY);
        let mono = FontId::monospace(crate::theme::MONO);
        let v = ui.visuals().clone();
        // The header, outside the scroll so it stays put.
        let (head, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
        for (c, r) in cols.cells(head) {
            cell(ui.painter(), r, c.title(), &font, v.strong_text_color());
        }
        ui.painter().hline(
            head.x_range(),
            head.bottom(),
            egui::Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
        );
        let mut pick = None;
        let nets = self.networks();
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(
            ui,
            row_h,
            self.alive.len(),
            |ui, range| {
                for i in range {
                    let h = &self.alive[i];
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                    let on = self.picked == Some(h.ip);
                    let fill = if on {
                        theme::accent().gamma_multiply(0.16)
                    } else if resp.hovered() {
                        v.widgets.hovered.weak_bg_fill
                    } else if i % 2 == 1 {
                        v.faint_bg_color
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    ui.painter()
                        .rect_filled(rect, crate::theme::R_CONTROL, fill);
                    let (text, weak) = (v.text_color(), v.weak_text_color());
                    for (c, r) in cols.cells(rect) {
                        let (s, f, colour) = match c {
                            Col::Address => (h.ip.to_string(), &mono, text),
                            Col::Time => match h.rtt {
                                Some(t) => (widgets::fmt_ms(t), &mono, text),
                                None if h.via.ais => ("imported".to_string(), &font, weak),
                                None => ("no ping".to_string(), &font, weak),
                            },
                            Col::Ttl => (
                                h.ttl.map(|t| t.to_string()).unwrap_or_default(),
                                &mono,
                                text,
                            ),
                            Col::Mac => (
                                h.mac.as_ref().map(mac::fmt).unwrap_or_default(),
                                &mono,
                                weak,
                            ),
                            Col::Vendor => (self.vendor(h), &font, text),
                            Col::Name => (h.name.clone().unwrap_or_default(), &font, text),
                            Col::Ports
                                if !h.checked
                                    && self.job.is_some()
                                    && self.port_queue.contains(&h.ip) =>
                            {
                                ("checking…".into(), &font, weak)
                            }
                            Col::Ports if !h.checked => ("not checked".into(), &font, weak),
                            Col::Ports => (port_list(&h.open), &mono, text),
                        };
                        cell(ui.painter(), r, &s, f, colour);
                    }
                    let resp = if h.via.ais {
                        resp.on_hover_text(AIS_TIP)
                    } else if h.via.multicast_only() {
                        resp.on_hover_text(MULTICAST_TIP)
                    } else if h.rtt.is_none() {
                        resp.on_hover_text(ARP_TIP)
                    } else {
                        resp
                    };
                    if resp.clicked() {
                        pick = Some((!on).then_some(h.ip));
                    }
                    // Right-click: its copies (only what it has), then
                    // Wake, as on a Map card.
                    let copies = row_copies(h, &self.vendor(h));
                    let wake = wake_target(h, &nets);
                    resp.context_menu(|ui| {
                        for (what, value) in &copies {
                            let b = egui::Button::new(format!("Copy {what}"))
                                .shortcut_text(widgets::short(value, 24))
                                .min_size(vec2(0.0, theme::TARGET_H));
                            if ui.add(b).clicked() {
                                widgets::copy(ui.ctx(), value.clone());
                                widgets::toast("Copied to the clipboard", true);
                                ui.close();
                            }
                        }
                        if let Some(t) = &wake {
                            ui.separator();
                            if ui
                                .add(
                                    egui::Button::new("Wake (Wake-on-LAN)")
                                        .min_size(vec2(0.0, theme::TARGET_H)),
                                )
                                .on_hover_text(wake_tip(t))
                                .clicked()
                            {
                                wol::wake_all(vec![*t]);
                                ui.close();
                            }
                        }
                    });
                }
            },
        );
        if let Some(p) = pick {
            self.picked = p;
        }
    }

    /// The picked host in the inspector; what its buttons asked for.
    fn inspector(&mut self, ui: &mut Ui) -> Option<crate::inspector::Ask> {
        use crate::inspector::{self as insp, Act, Ask};
        let h = self.alive.iter().find(|h| Some(h.ip) == self.picked)?;
        let ip = h.ip;
        let kind = if self.local.gateway == Some(ip) {
            "Gateway".to_string()
        } else if self.local.lan.is_some_and(|(me, _)| me == ip) {
            "This machine".to_string()
        } else if h.via.ais {
            "Host · imported".to_string()
        } else if h.via.multicast_only() {
            format!("Host · {} only", h.via.multicast())
        } else if h.rtt.is_none() {
            "Host · ARP only".to_string()
        } else {
            "Host".to_string()
        };
        let via = h.via;
        let upnp = h.ssdp.clone();
        let title = h.name.clone().unwrap_or_else(|| ip.to_string());
        let named = h.name.is_some();
        let mac = h.mac.as_ref().map(mac::fmt);
        let vendor = self.vendor(h);
        let (name_from, mac_from) = (h.name_from, h.mac_from);
        let (rtt, ttl, ports_open) = (h.rtt, h.ttl, h.open.clone());
        let web = insp::web_url(ip, &ports_open);
        let (checked, ran) = (h.checked, self.ran_ports);
        let web_title = h.web.clone();
        let cert = h.cert.as_deref().cloned();
        let found = h.dnssd.clone();
        let scanned = h.ais.clone();
        let scan_date = self.imported.as_ref().and_then(|i| i.date.clone());
        let wake = wake_target(h, &self.networks());
        let wake_tip = wake.as_ref().map_or_else(
            || {
                "No MAC to wake it by: Wake-on-LAN needs the card's real MAC, from ARP on this \
                 network or NetBIOS (a private MAC doesn't wake)"
                    .to_string()
            },
            wake_tip,
        );
        let also: Vec<String> = h
            .also
            .iter()
            .map(|m| {
                format!("{} {}", mac::fmt(m), map_vendor_label(m))
                    .trim_end()
                    .to_string()
            })
            .collect();
        let (snmp_ok, snmp_tip) = crate::tools::snmp::button(ip.into());
        let mut open = true;
        let mut ask = None;
        let address = ip.to_string();
        insp::show(ui, "sweep_inspector", &mut open, |ui, open| {
            insp::header(
                ui,
                &kind,
                &title,
                named.then_some(address.as_str()),
                Some(open),
            );
            let web_tip = format!("Open {web} in the browser");
            let acts = [
                Act {
                    label: "Ping",
                    tip: "Ping it in the Ping tab",
                    enabled: true,
                },
                Act {
                    label: "Ports",
                    tip: "Check its ports in the Port Check tab",
                    enabled: true,
                },
                Act {
                    label: "Trace",
                    tip: "Trace the route to it in the Traceroute tab",
                    enabled: true,
                },
                Act {
                    label: "Web",
                    tip: &web_tip,
                    enabled: true,
                },
                Act {
                    label: "On map",
                    tip: "Select it on the Map",
                    enabled: true,
                },
                Act {
                    label: "Domain join",
                    tip: "Put it in Domain join's Other computers box, to ask whether it's \
                          joined to Active Directory. Nothing is sent until you press Start",
                    enabled: true,
                },
                Act {
                    label: "Wake",
                    tip: &wake_tip,
                    enabled: wake.is_some(),
                },
                Act {
                    label: "SNMP",
                    tip: &snmp_tip,
                    enabled: snmp_ok,
                },
            ];
            match insp::actions(ui, &acts) {
                Some(0) => ask = Some(Ask::Ping(address.clone())),
                Some(1) => ask = Some(Ask::Ports(address.clone())),
                Some(2) => ask = Some(Ask::Trace(address.clone())),
                Some(3) => insp::open(web.clone()),
                Some(4) => ask = Some(Ask::OnMap(ip)),
                Some(5) => ask = Some(Ask::Join(address.clone())),
                Some(6) => {
                    if let Some(t) = wake {
                        wol::wake_all(vec![t]);
                    }
                }
                Some(7) => crate::tools::snmp::start(ui.ctx(), ip.into()),
                _ => {}
            }
            // Under the buttons, so what SNMP read shows where it was asked.
            crate::tools::snmp::section(ui, ip.into());
            insp::section(ui, "Details");
            insp::row(ui, "Address", &address, true);
            if let Some(m) = &mac {
                insp::row(ui, "MAC", m, true);
                if mac_from == Some("netbios") {
                    insp::row(ui, "MAC from", "the device, over NetBIOS", false);
                } else if mac_from == Some("ais") {
                    insp::row(ui, "MAC from", "Advanced IP Scanner", false);
                }
            }
            if !vendor.is_empty() {
                insp::row(ui, "Vendor", &vendor, false);
            }
            if let Some(t) = &web_title {
                insp::row(ui, "Web page", t, false);
            }
            if let Some(c) = &cert {
                cert_rows(ui, c);
            }
            insp::row(ui, "Found by", &via.words(), false);
            if via.multicast_only() {
                insp::note(ui, MULTICAST_NOTE);
            }
            if let Some(a) = &scanned {
                if let Some(d) = &scan_date {
                    insp::row(ui, "Scanned", d, false);
                }
                for (k, v) in [
                    ("OS", &a.os),
                    ("NetBIOS", &a.netbios_name),
                    ("Workgroup", &a.netbios_group),
                    ("TLS", &a.tls),
                    ("HTTPS server", &a.https),
                    ("FTP", &a.ftp),
                    ("RDP", &a.rdp),
                ] {
                    if let Some(v) = v {
                        insp::row(ui, k, v, false);
                    }
                }
                let sh = shared(a);
                if !sh.is_empty() {
                    insp::row(ui, "Shared", &format!("{sh} (names not imported)"), false);
                }
                insp::note(ui, AIS_NOTE);
            }
            if let Some(f) = &found {
                if let Some(m) = f.model() {
                    insp::row(ui, "Model", m, false);
                }
                if !f.rp.is_empty() {
                    insp::row(ui, "Queue", &f.rp.join(", "), true);
                }
                if !f.pdl.is_empty() {
                    insp::row(ui, "Languages", &languages(&f.pdl), false);
                }
                if !f.services.is_empty() {
                    insp::row(ui, "Services", &f.services.join(" "), true);
                }
                insp::note(ui, DNSSD_NOTE);
            }
            if let Some(u) = &upnp {
                if let Some(sv) = &u.server {
                    insp::row(ui, "UPnP", sv, false);
                }
                let kinds = upnp_kinds(&u.types);
                if !kinds.is_empty() {
                    insp::row(ui, "UPnP types", &kinds, false);
                }
                insp::note(ui, SSDP_NOTE);
            }
            for m in &also {
                insp::row(ui, "Also from", m, true);
            }
            if !also.is_empty() {
                insp::note(
                    ui,
                    "⚠ This address answered from more than one MAC during the sweep: two \
                     devices may be using it (a duplicate IP), or it failed over mid-sweep.",
                );
            }
            if named {
                insp::row(ui, "Name", &title, false);
                let from = match name_from {
                    Some("dns") => "DNS",
                    Some("netbios") => "the device, over NetBIOS",
                    Some("mdns") => "the device, over multicast DNS",
                    Some("ais") => "Advanced IP Scanner",
                    _ => "",
                };
                if !from.is_empty() {
                    insp::row(ui, "Name from", from, false);
                }
            }
            match rtt {
                Some(t) => insp::row(ui, "Ping", &widgets::fmt_ms(t), true),
                None if via.ais => insp::row(
                    ui,
                    "Ping",
                    "not pinged from here; Advanced IP Scanner found it alive",
                    false,
                ),
                None if via.arp => insp::row(ui, "Ping", "no reply; it answered ARP", false),
                None => insp::row(
                    ui,
                    "Ping",
                    &format!("no reply; it answered {} only", via.multicast()),
                    false,
                ),
            }
            if let Some(t) = ttl {
                insp::row(ui, "TTL", &t.to_string(), true);
            }
            insp::section(ui, "Open ports");
            if via.ais && !checked {
                insp::note(
                    ui,
                    "Advanced IP Scanner didn't record its service checks for this host. Use \
                     Ports to check it from here.",
                );
            } else if via.ais && ports_open.is_empty() {
                insp::note(
                    ui,
                    "Advanced IP Scanner found none of FTP, HTTP, HTTPS or RDP. It checks only \
                     those: use Ports for the rest.",
                );
            } else if !checked && ran == PortCheck::Smart {
                insp::note(
                    ui,
                    "Not checked: Smart skips phones and laptops (a private MAC, or a laptop or \
                     phone network card's maker). Use Ports, or sweep with Mgmt ports: All hosts.",
                );
            } else if !checked {
                insp::note(
                    ui,
                    "Not checked: set Mgmt ports (Options) to Smart or All hosts before the \
                     sweep, or use Ports.",
                );
            } else if ports_open.is_empty() {
                insp::note(ui, "None of the management ports answered.");
            } else {
                let chips: Vec<String> = ports_open
                    .iter()
                    .map(|p| match ports::service_name(*p) {
                        "" => p.to_string(),
                        s => format!("{p} {s}"),
                    })
                    .collect();
                insp::chips(ui, &chips);
                if via.ais {
                    insp::note(
                        ui,
                        "As Advanced IP Scanner found them: it checks FTP, HTTP, HTTPS and RDP \
                         only.",
                    );
                }
            }
        });
        if !open {
            self.picked = None;
        }
        ask
    }

    /// Hosts that answered ARP but not ping.
    fn arp_only(&self) -> usize {
        self.alive
            .iter()
            .filter(|h| h.via.arp && !h.via.ping)
            .count()
    }

    /// Hosts that answered only the multicast browse (DNS-SD or SSDP).
    fn multicast_only(&self) -> usize {
        self.alive.iter().filter(|h| h.via.multicast_only()).count()
    }

    /// Who made a host by its MAC (`map::maker`): the gateway's MAC is
    /// never private; a gear maker's virtual MAC is named when the host
    /// showed gear of its own (a gear port or page).
    fn maker(&self, h: &Host) -> Option<crate::map::Maker> {
        let gateway = self.local.gateway == Some(h.ip);
        let open: &[u16] = if h.checked { &h.open } else { &[] };
        let gear = crate::map::gear_evidence(open, h.web.as_deref());
        h.mac.map(|m| crate::map::maker(&m, gateway, gear))
    }

    /// Vendor (an imported host's maker as Advanced IP Scanner named it,
    /// else the registry's), or what the host is to this machine when it
    /// has no MAC here.
    fn vendor(&self, h: &Host) -> String {
        if let Some(m) = h.ais.as_ref().and_then(|a| a.maker.clone()) {
            return m;
        }
        match self.maker(h) {
            Some(m) => crate::map::vendor_label(m),
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
        let mut s = match &self.imported {
            Some(im) => format!(
                "Advanced IP Scanner scan of {}{}, imported: {} alive",
                self.ran,
                im.date
                    .as_ref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default(),
                self.alive.len()
            ),
            None => format!("Sweep of {}: {} alive", self.ran, self.alive.len()),
        };
        let mut only = Vec::new();
        match self.arp_only() {
            0 => {}
            n => only.push(format!("{n} found by ARP only, no ping reply")),
        }
        match self.multicast_only() {
            0 => {}
            n => only.push(format!(
                "{n} found by mDNS or SSDP only, no ping or ARP reply"
            )),
        }
        if only.is_empty() {
            s += "\n";
        } else {
            s += &format!(" ({})\n", only.join("; "));
        }
        let dup = self.alive.iter().filter(|h| !h.also.is_empty()).count();
        if dup > 0 {
            s += &format!(
                "⚠ {dup} address(es) answered from more than one MAC: a duplicate IP, or a \
                 failover during the sweep\n"
            );
        }
        let selfie = self.alive.iter().filter(|h| h.ip.is_link_local()).count();
        if selfie > 0 {
            s += &format!(
                "⚠ {selfie} self-assigned address(es) (169.254.x.x): {}\n",
                crate::map::NO_DHCP
            );
        }
        if let Some(im) = &self.imported {
            s += &format!(
                "Not imported: {} dead and {} unknown (Advanced IP Scanner's status); only \
                 alive rows become hosts\n",
                im.dead, im.unknown
            );
            if im.skipped > 0 {
                s += &format!(
                    "{} row(s) skipped: no IPv4 address, or a second row for one address\n",
                    im.skipped
                );
            }
            if im.cut {
                s += &format!(
                    "⚠ More than {} rows: the rest weren't read\n",
                    ais::MAX_ROWS
                );
            }
            s += "Services as Advanced IP Scanner found them (FTP, HTTP, HTTPS, RDP only). \
                  Logged-on users aren't imported; shares and printers are counted, not named\n";
        }
        if self.stopped {
            s += "Stopped before it finished: hosts it hadn't reached aren't listed, and ports \
                  it hadn't checked say not checked\n";
        }
        if self.browsed || self.searched {
            let mut said = Vec::new();
            if self.browsed {
                let n = self.alive.iter().filter(|h| h.dnssd.is_some()).count();
                said.push(format!("DNS-SD browse: {n} answered"));
            }
            if self.searched {
                let n = self.alive.iter().filter(|h| h.ssdp.is_some()).count();
                said.push(format!("SSDP search: {n} answered"));
            }
            s += &format!(
                "{}; {} found only this way (this computer's own segment only; routed \
                 networks can't answer it)\n",
                said.join(", "),
                self.multicast_only()
            );
        }
        let checked = self.alive.iter().filter(|h| h.checked).count();
        match self.ran_ports {
            // Nothing alive: nothing to say about ports.
            _ if self.alive.is_empty() => {}
            PortCheck::Off => {}
            PortCheck::Smart => {
                s += &format!(
                    "Mgmt ports checked on {checked} of {} (Smart: phones and laptops skipped)\n",
                    self.alive.len()
                )
            }
            PortCheck::All => s += &format!("Mgmt ports checked on all {checked}\n"),
        }
        for h in &self.alive {
            let rtt = match h.rtt {
                Some(r) => format!("{r:.1} ms"),
                None if h.via.ais => "imported".into(),
                None if h.via.arp => "ARP only".into(),
                None => match (h.via.mdns, h.via.ssdp) {
                    (true, true) => "mDNS+SSDP".into(),
                    (true, false) => "mDNS only".into(),
                    _ => "SSDP only".into(),
                },
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
            if let Some(t) = &h.web {
                line += &format!("  \"{t}\"");
            }
            if let Some(c) = h.cert.as_deref() {
                line += &cert_words(c, h.titled, chrono::Utc::now());
            }
            if let Some(m) = h.dnssd.as_deref().and_then(dnssd::Found::model) {
                line += &format!("  model \"{m}\"");
            }
            if let Some(sv) = h.ssdp.as_deref().and_then(|u| u.server.as_deref()) {
                line += &format!("  UPnP \"{sv}\"");
            }
            if let Some(a) = &h.ais {
                if let Some(n) = a
                    .netbios_name
                    .as_ref()
                    .filter(|n| h.name.as_ref() != Some(*n))
                {
                    line += &format!("  NetBIOS {n}");
                }
                if let Some(g) = &a.netbios_group {
                    line += &format!("  workgroup {g}");
                }
                if let Some(os) = &a.os {
                    line += &format!("  OS \"{os}\"");
                }
                if let Some(t) = &a.tls {
                    line += &format!("  {t}");
                }
                if let Some(f) = &a.ftp {
                    line += &format!("  FTP \"{f}\"");
                }
                if let Some(r) = &a.rdp {
                    line += &format!("  RDP \"{r}\"");
                }
                let sh = shared(a);
                if !sh.is_empty() {
                    line += &format!("  {sh}");
                }
            }
            if !h.also.is_empty() {
                let also: Vec<String> = h.also.iter().map(mac::fmt).collect();
                line += &format!("  ⚠ also from {}", also.join(", "));
            }
            if h.ip.is_link_local() {
                line += "  ⚠ self-assigned, no DHCP";
            }
            line += &format!("  (found by {})", h.via.list().join(", "));
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
                    self.ran.clone(),
                    h.ip.to_string(),
                    h.rtt.map(log::ms).unwrap_or_default(),
                    log::cell(h.ttl),
                    h.mac.as_ref().map(mac::fmt).unwrap_or_default(),
                    self.vendor(h),
                    h.name.clone().unwrap_or_default(),
                    if h.checked || !self.checked_ports() {
                        port_list(&h.open)
                    } else {
                        log::NOT_CHECKED.into()
                    },
                    found_by(h).join(" "),
                ]
            })
            .collect();
        Some(Table {
            columns: COLUMNS,
            rows,
        })
    }

    /// Site inventory for /mapnetwork: facts only, no role guesses.
    /// The live hosts as JSON for /mapnetwork. `clients` says what
    /// becomes of the names of hosts that look like a phone, laptop or PC
    /// (the Site Bundle's setting; Full for the tech's own save).
    pub fn inventory(&self, info: &HostInfo, clients: Clients) -> String {
        // The segment the scanner sat on when the run started. An imported
        // scan ran on another computer, whose segment isn't known.
        let lan = match self.imported {
            Some(_) => None,
            None => self.local.lan.or(info.lan),
        };
        let hosts: Vec<_> = self
            .alive
            .iter()
            .map(|h| {
                let open: &[u16] = if h.checked { &h.open } else { &[] };
                let words = h.cert.as_deref().map(tls::Cert::words);
                let said = crate::map::Said {
                    page: h.web.as_deref(),
                    dnssd: h.dnssd.as_deref(),
                    cert: words.as_deref(),
                };
                let client = self.local.gateway != Some(h.ip)
                    && crate::map::looks_like_client(h.mac.as_ref(), open, said);
                let key = h.mac.as_ref().map_or_else(|| h.ip.to_string(), mac::fmt);
                let made = self.maker(h);
                json!({
                    "ip": h.ip.to_string(),
                    "mac": h.mac.as_ref().map(mac::fmt),
                    "vendor": h
                        .ais
                        .as_ref()
                        .and_then(|a| a.maker.clone())
                        .or_else(|| made.and_then(|m| m.vendor).map(str::to_string)),
                    "private_mac": made.map(|m| m.private),
                    "virtual_mac": made.map(|m| m.virtual_mac),
                    "name": clients.device_name(h.name.as_deref(), &key, client),
                    "name_from": h.name_from,
                    "mac_from": h.mac_from,
                    "found_by": found_by(h),
                    "rtt_ms": h.rtt.map(|r| (r * 100.0).round() / 100.0),
                    "ttl": h.ttl,
                    "open_ports": h.checked.then(|| h.open.iter().map(|&p| json!({
                        "port": p,
                        "service": ports::service_name(p),
                    })).collect::<Vec<_>>()),
                    "other_macs": h.also.iter().map(mac::fmt).collect::<Vec<_>>(),
                    "web_title": h.web,
                    "tls": h.cert.as_deref().map(|c| json!({
                        "subject": c.dn(),
                        "sans": c.names,
                        "issuer": c.issuer,
                        "not_after": c.not_after.map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string()),
                        "self_signed": c.self_signed,
                    })),
                    "dnssd": h.dnssd.as_ref().map(|f| json!({
                        "services": f.services,
                        "ty": f.ty,
                        "product": f.product,
                        "rp": f.rp,
                        "pdl": f.pdl,
                    })),
                    "ssdp": h.ssdp.as_ref().map(|u| json!({
                        "server": u.server,
                        "st": u.types,
                        "uuids": u.uuids,
                        "locations": u.locations,
                    })),
                    "ais": h.ais.as_ref().map(|a| json!({
                        "netbios_name": clients.device_name(a.netbios_name.as_deref(), &key, client),
                        "netbios_group": a.netbios_group,
                        "os": a.os,
                        "tls": a.tls,
                        "https_server": a.https,
                        "ftp": a.ftp,
                        "rdp": a.rdp,
                        "shares": a.shares,
                        "printers": a.printers,
                    })),
                    "is_gateway": self.local.gateway == Some(h.ip),
                    "is_this_machine": self.local.lan.is_some_and(|(ip, _)| ip == h.ip),
                    "on_scanner_lan": lan.map(|(ip, bits)| same_subnet(ip, h.ip, bits)),
                })
            })
            .collect();
        let im = self.imported.as_ref();
        let here = im.is_none();
        let v = json!({
            "source": match im {
                Some(_) => format!("is-alive {} import of an Advanced IP Scanner scan", crate::VERSION),
                None => format!("is-alive {} subnet sweep", crate::VERSION),
            },
            "scanned_at": im
                .and_then(|i| i.date.clone())
                .or_else(|| self.run_started.clone())
                .unwrap_or_else(log::now),
            "range": self.ran,
            "ranges": targets::list(&self.ran),
            // An imported scan ran on a computer whose network isn't known.
            "scanner": here.then(|| json!({
                "lan": lan.map(|(ip, bits)| format!("{ip}/{bits}")),
                "gateway": self.local.gateway.or(info.gateway).map(|g| g.to_string()),
                "wan": info.wan.map(|w| w.to_string()),
            })),
            "imported": im.map(|i| json!({
                "from": "Advanced IP Scanner",
                "scanned_at": i.date,
                "dead": i.dead,
                "unknown": i.unknown,
                "skipped": i.skipped,
            })),
            "ports_checked": if im.is_some() {
                ais::PORTS
            } else if self.ran_ports == PortCheck::Off {
                &[]
            } else {
                ports::INFRA
            },
            "client_names": clients.label(),
            "dnssd_browsed": self.browsed,
            "ssdp_searched": self.searched,
            "complete": !self.stopped && !im.is_some_and(|i| i.cut),
            "notes": [
                "found_by lists every way the host answered: ping (an echo reply, rtt_ms set), arp (this machine's ARP table confirmed it during the sweep, on the scanner's own segment), mdns (it answered the DNS-SD browse) and ssdp (it answered the SSDP search). Several are usual: a printer is often [\"ping\", \"arp\", \"mdns\"]. Without ping, rtt_ms is null: [\"arp\"] answered ARP but drops ping (a firewall default); a list with only mdns and/or ssdp is a device silent to ping and ARP that answered the multicast browse, listed only because it answered from its own address, inside the swept range and on the scanner's own segment (on_scanner_lan true); an address merely named in another device's answer is never added. A host that answers none of these is missed, so absence is not proof.",
                "mac_from says where mac came from: arp (this machine's ARP table, hosts on the scanner's own segment) or netbios (the host reported it, a Windows host past a router). Otherwise mac is null: past a router that is expected, not a sign of an unknown device.",
                "name_from says where name came from: dns (its PTR record), netbios (the computer name a Windows, Samba or NAS host reports) or mdns (a name.local an Apple device, printer, cast device or Linux host reports). netbios and mdns are the device's own claim: good labels and role hints, not proof of identity.",
                "client_names says what became of the names of hosts that look like a phone, laptop or PC (not the gateway, and no maker, port or web page that says network gear, NAS, printer or camera), names that are often people's: full keeps them; anonymized replaces each with a client-xxxxxxxx token made from its MAC (else its IP), the same token the UniFi export's client table uses for that MAC; none leaves name null. name_from still says where the real name came from. Other hosts keep their names.",
                "ranges lists each network swept; on_scanner_lan says whether a host is on the scanner's own segment (true) or routed (false: a mac only if the host reported one over NetBIOS; null when the scanner's LAN wasn't known).",
                "vendor is the IEEE registry owner of the MAC prefix (null if unregistered). A locally administered MAC (second-lowest bit of the first octet set) is either a client's private address or a device's virtual one: virtual_mac true means it is the gateway's (never private, whatever its MAC) or a network-gear maker's own prefix with that bit set, such as a SonicWall LAN interface answering on 2e:b8:ed:… for SonicWall's 2c:b8:ed:…, on a host that showed gear of its own (telnet, 541, 4433 or 8291 open, or a web page naming gear; the Smart port check skips locally administered MACs, so off the gateway that needs All hosts); vendor then names that maker (null for a gateway whose maker isn't known). private_mac true means a locally administered address with nothing to say infrastructure (a phone or laptop's private per-network MAC, or a VM's virtual NIC), vendor null. Both are false for a registered MAC and null when mac is null. HP's prefixes cover HP printers, PCs and Poly desk phones alike: tell them apart by web_title, dnssd and open_ports, not the vendor.",
                "web_title is what the host's own web page calls it (its <title>, else its Server header; a placeholder title such as Loading..., Login or Redirecting counts as none), read with one GET where a management web port answered: the device's own claim, a strong role hint (a UniFi console, a SonicWall login, an HP printer's page), not proof.",
                "tls is the certificate the host showed on its HTTPS management port (443, 8443 or 4433), read from the same handshake as the web_title GET (or, where no GET went over TLS, one handshake that is then closed), null where no handshake finished: subject is its subject as CN=…, OU=…, O=… (the parts it has; null if none), sans its DNS names and IP addresses (at most 16), issuer its issuer's common name (else organisation), not_after its end date (UTC, null if unreadable) and self_signed whether it issued itself. Nothing about it is trusted or checked. Device certificates often carry the model or serial (a FortiGate's FGT60F… serial, a NAS or printer's model, a camera maker): a role and model hint where web_title is null or only a Server header, the device's own claim, not proof. A self-signed or expired certificate is usual on a management page: a low note at most, never a finding by itself.",
                "dnssd is what the host said over DNS-SD (Bonjour/multicast DNS), null if it said nothing: services lists the service types it offers (_ipp._tcp IPP printing, _pdl-datastream._tcp raw printing, _airplay._tcp, _googlecast._tcp, _smb._tcp file sharing, _http._tcp a web page and so on); ty and product are a printer's make and model from its TXT record (product without its brackets), null when it didn't give one or the host advertised several different printers; rp lists its print queues (rp=, such as ipp/print; a print server has one per printer) and pdl the page languages its printer services accept (pdl=, MIME types such as application/pdf or application/vnd.hp-PCL), both [] when not given. A model belongs to the printer service: a computer or print server that shares a printer advertises that printer's model, so check the rest of the host's evidence before calling it a printer. The device's own claim: a strong model and role hint, not proof. dnssd_browsed (top level) says whether the browse ran and its answers were read: false with names off, when no swept range is on the scanner's segment, or when the sweep was stopped first. Multicast DNS stays on the scanner's own segment, so only hosts with on_scanner_lan true can answer; null on a routed host is expected, not a sign it has no services.",
                "ssdp is what the host said over SSDP (UPnP discovery), null if it said nothing: server is its SERVER header (its OS, UPnP stack and often product, such as a NAS's or a TV's), st lists the search targets it answered for (upnp:rootdevice, urn:schemas-upnp-org:device:MediaRenderer:1, urn:schemas-upnp-org:device:InternetGatewayDevice:1 and so on: its device and service types), uuids the device UUIDs from its USN headers, and locations the URL of its description document as text: is-alive never fetches it. The device's own claim: a role hint (a media renderer, a NAS, an internet gateway with UPnP on), not proof. ssdp_searched (top level) says whether the search ran and its replies were read (false in the same cases as dnssd_browsed); it is sent with TTL 1, and like DNS-SD it is multicast and only the scanner's own segment answers, so null on a routed host is expected.",
                "other_macs lists any other MAC the address answered from during the sweep (each a fresh or confirmed ARP entry): two devices on one IP (a duplicate address), or a failover mid-sweep.",
                "An ip in 169.254.0.0/16 is self-assigned: DHCP didn't answer that device (check the DHCP server/scope and the switch port). ports_checked includes 21 (ftp): printers and NAS often leave FTP on, which sends logins in clear text.",
                "open_ports is null when that host's ports were not checked: the default Smart check skips phones and laptops (private_mac true, or a laptop or phone network card's maker such as Intel, Realtek, Apple or Samsung), and a stopped sweep leaves the hosts it hadn't finished. Null is not \"none open\"; an empty list is.",
                "complete false means the sweep was stopped before it finished: a host missing from hosts may simply not have been reached, so absence says even less than usual.",
                "imported is null for a sweep is-alive ran itself. Otherwise the hosts come from an Advanced IP Scanner XML export the tech imported, scanned on another computer at imported.scanned_at (as that scanner wrote it; scanned_at matches): every host has found_by [\"ais\"] (Advanced IP Scanner found it alive; is-alive did not ping it, so rtt_ms and ttl are null), mac_from ais (the scanner's own reading), name_from ais (the name the scanner resolved, usually DNS) or netbios, and vendor is the maker the scanner named (else the IEEE registry's). Only rows it marked alive become hosts: imported.dead and imported.unknown count the rest, imported.skipped the rows with no IPv4 address and the second row for an address already read (an alive row wins over a dead or unknown one). imported.scanned_at is the newest row date that reads as a date, else the first row's, as written. range and ranges are every address a row named, not a range typed in: one per /24 (lowest to highest there, or the whole /24), summarised past 32 as one span per /16, and past 32 of those as one span from the lowest address to the highest, so a summarised span may include addresses the scan never tried; scanner and on_scanner_lan are null because the scanning computer's segment isn't known. open_ports is what its service flags found among ports_checked (FTP 21, HTTP 80 or 8080, HTTPS 443, RDP 3389: all it checks, so [] is not proof nothing else is open), null when the row carried no flags. ais holds the rest: netbios_name and netbios_group (the computer name and workgroup or domain it reported; netbios_name follows client_names), os, tls (the TLS or SSL version its HTTPS answered with: TLSv1 or SSLv3 is outdated), https_server, ftp and rdp (banners), and shares and printers (how many it shares: counts only, names are never imported). Logged-on users are never imported.",
            ],
            "hosts": hosts,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"
    }

    /// Beside the box: how many targets and addresses it lists, or what's
    /// wrong with it, read again only when the text changes.
    fn plan_label(&mut self, ui: &mut Ui) {
        let fresh = self.plan.as_ref().is_some_and(|(t, _)| *t == self.range);
        if !fresh {
            let r = targets::count(&self.range);
            self.plan = Some((self.range.clone(), r));
        }
        let Some((_, r)) = &self.plan else { return };
        match r {
            Ok((1, n)) => {
                ui.weak(format!(
                    "{} address{}",
                    subnet::group(*n as u128),
                    if *n == 1 { "" } else { "es" }
                ));
            }
            Ok((parts, n)) => {
                ui.weak(format!(
                    "{parts} targets · {} addresses",
                    subnet::group(*n as u128)
                ))
                .on_hover_text("Overlapping targets are swept once");
            }
            Err(_) if self.range.trim().is_empty() => {}
            Err(e) => {
                ui.add(
                    egui::Label::new(RichText::new(format!("⚠ {e}")).color(widgets::warn()))
                        .truncate(),
                )
                .on_hover_text(e.as_str());
            }
        }
    }

    /// For the screenshot tour: `spec` in the box, nothing swept.
    pub fn show_for_shots(&mut self, spec: &str) {
        self.range = spec.into();
        self.detected = true;
    }

    /// For the screenshot tour: a finished sweep of fictional hosts on a
    /// documentation network, the printer open in the inspector. Nothing
    /// is sent.
    pub fn sample_for_shots(&mut self) {
        self.clear_results();
        self.detected = true;
        self.range = "192.0.2.0/24".into();
        self.ran = self.range.clone();
        self.total = 254;
        self.probed = 254;
        self.run_started = None;
        self.browsed = true;
        let host = |last: u8, name: Option<&str>, mac: Option<Mac>| Host {
            ip: Ipv4Addr::new(192, 0, 2, last),
            rtt: Some(0.4 + last as f64 / 100.0),
            ttl: Some(64),
            name: name.map(str::to_string),
            name_from: name.map(|_| "mdns"),
            mac,
            mac_from: mac.map(|_| "arp"),
            open: Vec::new(),
            checked: false,
            also: Vec::new(),
            web: None,
            titled: false,
            cert: None,
            dnssd: None,
            ssdp: None,
            ais: None,
            via: Via {
                arp: mac.is_some(),
                ..Via::PING
            },
        };
        let mut printer = host(
            40,
            Some("NPI0A1B2C.local"),
            Some([0x00, 0x11, 0x32, 0x00, 0x00, 0x40]),
        );
        printer.dnssd = Some(Box::new(dnssd::Found {
            services: vec![
                "_http._tcp".into(),
                "_ipp._tcp".into(),
                "_pdl-datastream._tcp".into(),
            ],
            ty: Some("HP LaserJet Pro M404".into()),
            product: Some("HP LaserJet Pro M404".into()),
            rp: vec!["ipp/print".into()],
            pdl: vec![
                "application/pdf".into(),
                "application/vnd.hp-PCL".into(),
                "image/urf".into(),
            ],
        }));
        printer.via.mdns = true;
        // Its page's title was a placeholder: the Server header stands in,
        // and its certificate names it.
        (printer.open, printer.checked) = (vec![80, 443, 9100], true);
        printer.web = Some("HP HTTP Server".into());
        printer.cert = Some(Box::new(tls::Cert {
            subject: "NPI0A1B2C".into(),
            cn: Some("NPI0A1B2C".into()),
            unit: Some("Jetdirect".into()),
            issuer: "NPI0A1B2C".into(),
            names: vec!["NPI0A1B2C.local".into(), "192.0.2.40".into()],
            not_after: chrono::NaiveDate::from_ymd_opt(2034, 1, 1)
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|t| t.and_utc()),
            self_signed: true,
            ..Default::default()
        }));
        // Silent to ping and ARP: found only by its multicast answers.
        let mut tv = host(75, Some("Lobby-TV.local"), None);
        (tv.rtt, tv.ttl, tv.via) = (
            None,
            None,
            Via {
                mdns: true,
                ssdp: true,
                ..Via::default()
            },
        );
        tv.dnssd = Some(Box::new(dnssd::Found {
            services: vec!["_googlecast._tcp".into(), "_http._tcp".into()],
            ..Default::default()
        }));
        tv.ssdp = Some(Box::new(ssdp::Found {
            server: Some("Linux/4.9 UPnP/1.0 ExampleCast/1.0".into()),
            types: vec![
                "upnp:rootdevice".into(),
                "urn:dial-multiscreen-org:device:dial:1".into(),
                "urn:schemas-upnp-org:device:MediaRenderer:1".into(),
            ],
            uuids: vec!["uuid:00000000-0000-4000-8000-000000000075".into()],
            locations: vec!["http://192.0.2.75:8008/ssdp/device-desc.xml".into()],
        }));
        self.alive = vec![
            host(1, None, Some([0x00, 0x11, 0x32, 0x00, 0x00, 0x01])),
            host(
                20,
                Some("FRONTDESK-PC"),
                Some([0x00, 0x11, 0x32, 0x00, 0x00, 0x20]),
            ),
            printer,
            tv,
        ];
        self.picked = Some(Ipv4Addr::new(192, 0, 2, 40));
        self.revision += 1;
    }

    /// For the screenshot tour: a fictional Advanced IP Scanner export
    /// imported (documentation addresses), the PC open in the inspector.
    /// Nothing is sent.
    pub fn sample_import_for_shots(&mut self) {
        if let Ok(scan) = ais::parse(ais::SAMPLE.as_bytes()) {
            self.load_import("example-scan.xml", scan);
        }
        self.run_started = None;
        self.picked = Some(Ipv4Addr::new(192, 0, 2, 20));
    }

    /// For the screenshot tour: the sample sweep with the host at
    /// 192.0.2.`last` open in the inspector.
    pub fn pick_for_shots(&mut self, last: u8) {
        self.picked = Some(Ipv4Addr::new(192, 0, 2, last));
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

    /// Import scan…'s file dialog is open, or the file is being read: what
    /// it brings would land in whatever site is open by then.
    pub fn importing(&self) -> bool {
        self.importing.is_some()
    }

    /// The last run is an imported scan from another computer.
    pub fn imported(&self) -> bool {
        self.imported.is_some()
    }

    /// Import scan…: pick an Advanced IP Scanner XML export and read it,
    /// off the UI thread. Nothing goes on the network.
    fn import_dialog(&mut self, ctx: &egui::Context) {
        if self.importing.is_some() {
            return;
        }
        self.err = None;
        self.importing = Some(Job::spawn(ctx, |tx| {
            let picked = rfd::FileDialog::new()
                .set_title("Import an Advanced IP Scanner scan")
                .add_filter("Advanced IP Scanner XML export", &["xml"])
                .add_filter("All files", &["*"])
                .pick_file();
            if let Some(p) = picked {
                tx.send(read_import(&p));
            }
        }));
    }

    /// An Advanced IP Scanner export as the last run, in place of a
    /// sweep: its alive rows as hosts, the dead and unknown ones counted.
    /// It reaches the Map, Results, the report and the inventory as a
    /// sweep does. `file` is shown on the tab only.
    pub fn load_import(&mut self, file: &str, scan: ais::Scan) {
        self.clear_results();
        self.runs += 1;
        self.detected = true;
        self.ran = scan.ranges.clone();
        self.total = scan.rows();
        self.probed = self.total;
        self.run_started = Some(log::now());
        self.imported = Some(Imported {
            file: file.to_string(),
            date: scan.date.clone(),
            dead: scan.dead,
            unknown: scan.unknown,
            skipped: scan.skipped,
            cut: scan.cut,
        });
        self.alive = scan.hosts.into_iter().filter_map(imported_host).collect();
        self.alive.sort_by_key(|h| h.ip);
        self.revision += 1;
    }

    /// While running: hosts probed, how many in all, since when, and the range.
    pub fn progress(&self) -> Option<(usize, usize, Instant, &str)> {
        self.job
            .is_some()
            .then_some((self.probed, self.total, self.started_at, self.ran.as_str()))
    }

    /// Range and live-host count of a finished run, if there is one.
    pub fn finished(&self) -> Option<(String, usize)> {
        (self.total > 0 && self.job.is_none()).then(|| (self.ran.clone(), self.alive.len()))
    }

    /// Where the last finished run was made from: this machine's LAN and
    /// gateway when it started (not now: the laptop may have moved since).
    pub fn scanned_from(&self) -> Option<&HostInfo> {
        self.finished().map(|_| &self.local)
    }

    /// Whether the last run checked management ports.
    pub fn checked_ports(&self) -> bool {
        self.ran_ports != PortCheck::Off || self.imported.is_some()
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
                checked: h.checked,
                also: h.also.clone(),
                web: h.web.clone(),
                dnssd: h.dnssd.as_deref().cloned(),
                cert: h.cert.as_deref().cloned(),
                titled: h.titled,
                multicast: h
                    .via
                    .multicast_only()
                    .then(|| h.via.multicast().to_string()),
                imported: h.via.ais.then(|| {
                    let date = self.imported.as_ref().and_then(|i| i.date.as_deref());
                    let mut s = crate::map::IMPORTED.to_string();
                    if let Some(d) = date {
                        s += &format!(" of {d}");
                    }
                    if let Some(os) = h.ais.as_ref().and_then(|a| a.os.as_deref()) {
                        s += &format!(", OS {os}");
                    }
                    s
                }),
            })
            .collect()
    }

    /// Changes whenever `seen` would.
    pub fn generation(&self) -> u64 {
        self.revision
    }

    /// Map → Clear map: stop a run and forget what was found. The ranges
    /// and options stay as set.
    pub fn clear_results(&mut self) {
        self.job = None;
        self.ran.clear();
        self.run_started = None;
        self.total = 0;
        self.probed = 0;
        self.alive.clear();
        self.local = HostInfo::default();
        self.ran_ports = PortCheck::Off;
        self.port_queue.clear();
        self.stopped = false;
        self.err = None;
        self.revision += 1;
        self.picked = None;
        self.browsed = false;
        self.searched = false;
        self.imported = None;
        // An import still being picked or read is dropped with the rest.
        self.importing = None;
    }

    /// The whole networks the last run swept (10.20.0.0/24, not a range
    /// like 10.20.0.10-50), for Wake's directed broadcast.
    pub fn networks(&self) -> Vec<(Ipv4Addr, u32)> {
        targets::networks(&self.ran)
    }

    /// Each host found with a MAC Wake can use: its name, MAC, address
    /// and the swept network holding it (Ping's Wake).
    pub fn wakeable(&self) -> Vec<(Option<String>, wol::Target)> {
        let nets = self.networks();
        self.alive
            .iter()
            .filter_map(|h| Some((h.name.clone(), wake_target(h, &nets)?)))
            .collect()
    }

    /// How many sweeps have started (each may be another network).
    pub fn runs(&self) -> u64 {
        self.runs
    }

    fn start(&mut self, ctx: &egui::Context) {
        // An import still being picked or read would cancel this sweep and
        // lose its Results entry when it lands: Start drops it.
        self.importing = None;
        // A range that won't read leaves the last sweep as it was: a typo
        // doesn't wipe 40 hosts.
        let ips = match targets::parse_range(&self.range) {
            Ok(v) => v,
            Err(e) => return self.err = Some(e),
        };
        self.err = None;
        self.stopped = false;
        self.port_queue.clear();
        self.revision += 1;
        self.runs += 1;
        self.alive.clear();
        self.probed = 0;
        self.ran = self.range.trim().to_string();
        self.total = ips.len();
        self.local = HostInfo::local();
        self.ran_ports = self.ports;
        self.browsed = false;
        self.searched = false;
        self.imported = None;
        // DNS-SD and SSDP only where they can be heard: with names on, and
        // only if the sweep covers part of this computer's own segment.
        let lan = self.local.lan;
        let browse = (self.resolve_names
            && lan.is_some_and(|(me, bits)| ips.iter().any(|&ip| same_subnet(me, ip, bits))))
        .then(|| lan.map(|l| l.0))
        .flatten();
        let (timeout, threads, rate, names, check_ports) = (
            self.timeout_ms,
            self.threads,
            self.rate,
            self.resolve_names,
            self.ports,
        );
        let gateway = self.local.gateway;

        self.run_started = Some(log::now());
        self.started_at = Instant::now();
        let me = self.local.lan.map(|(ip, _)| ip);
        // A device answers NetBIOS or mDNS within milliseconds on a LAN.
        let ask = Duration::from_millis(timeout.min(400) as u64);
        self.job = Some(Job::spawn(ctx, move |tx| {
            // Sent first and together, so devices answer while the pings
            // go out; read after the ping and ARP phases.
            let search = browse.and_then(|via| ssdp::send(Some(via), ssdp::GROUP).ok());
            let browse = browse.and_then(|via| dnssd::send(Some(via), dnssd::GROUP).ok());
            let alive = Mutex::new(Vec::new());
            // Each live host's MAC as found, for Smart's choice of hosts.
            let macs: Mutex<HashMap<Ipv4Addr, Mac>> = Mutex::new(HashMap::new());
            // Every MAC each address answered from (fresh or confirmed ARP
            // entries only): two for one address is a duplicate IP.
            let readings: Mutex<HashMap<Ipv4Addr, std::collections::BTreeSet<Mac>>> =
                Mutex::new(HashMap::new());
            let note = |ip: Ipv4Addr, m: Mac| {
                readings.lock().unwrap().entry(ip).or_default().insert(m);
            };
            // Each probe makes the OS ARP for the address, so hosts that drop
            // ping still land in the neighbor table as confirmed entries.
            let arp: Mutex<HashMap<Ipv4Addr, Mac>> = Mutex::new(HashMap::new());
            let last_look = Mutex::new(Instant::now());
            let look = || {
                let now = mac::confirmed();
                for (&ip, &m) in &now {
                    note(ip, m);
                }
                arp.lock().unwrap().extend(now);
            };
            net::par_for(&ips, threads, rate, tx.cancel_flag(), |&ip| {
                let r = icmp::echo(IpAddr::V4(ip), 128, timeout, 32);
                if r.is_reply() {
                    // The reply means the neighbor entry was just resolved.
                    let mac = mac::neighbors().get(&ip).copied();
                    let (name, name_from, own_mac) = if names {
                        identify(ip, mac.is_some(), ask)
                    } else {
                        (None, None, None)
                    };
                    let mac_from = match (mac, own_mac) {
                        (Some(_), _) => Some("arp"),
                        (None, Some(_)) => Some("netbios"),
                        (None, None) => None,
                    };
                    alive.lock().unwrap().push((ip, mac.is_some()));
                    if let Some(m) = mac.or(own_mac) {
                        macs.lock().unwrap().insert(ip, m);
                    }
                    // Read just as it replied: fresh, not a stale entry.
                    if let Some(m) = mac {
                        note(ip, m);
                    }
                    tx.send(Ev::Alive(Host {
                        ip,
                        rtt: Some(r.rtt_ms),
                        ttl: r.ttl,
                        name,
                        name_from,
                        mac: mac.or(own_mac),
                        mac_from,
                        open: Vec::new(),
                        checked: false,
                        also: Vec::new(),
                        web: None,
                        titled: false,
                        cert: None,
                        dnssd: None,
                        ssdp: None,
                        ais: None,
                        via: Via {
                            arp: mac.is_some(),
                            ..Via::PING
                        },
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
                    macs.lock().unwrap().insert(ip, *m);
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
                let (name, name_from, _) = if names {
                    identify(ip, true, ask)
                } else {
                    (None, None, None)
                };
                tx.send(Ev::Alive(Host {
                    ip,
                    rtt: None,
                    ttl: None,
                    name,
                    name_from,
                    mac: Some(m),
                    mac_from: Some("arp"),
                    open: Vec::new(),
                    checked: false,
                    also: Vec::new(),
                    web: None,
                    titled: false,
                    cert: None,
                    dnssd: None,
                    ssdp: None,
                    ais: None,
                    via: Via::ARP,
                }));
            });
            seen.extend(silent.iter().map(|s| s.0));
            let mut macs = macs.into_inner().unwrap();
            macs.extend(silent.iter().copied());

            // What answered the multicast discovery: labels for hosts
            // already found on this segment, and the hosts silent to ping
            // and ARP. Only a device's own UDP answer adds a host: an
            // address named in another's A record only labels one.
            let go = !tx.cancelled();
            let browse = browse.filter(|_| go);
            let search = search.filter(|_| go);
            let (browsed, searched) = (browse.is_some(), search.is_some());
            let (mut heard, answered) = browse
                .map(|b| b.collect(dnssd::WAIT, lan))
                .unwrap_or_default();
            let mut upnp = search.map(|s| s.collect(ssdp::WAIT)).unwrap_or_default();
            // Said only once the answers were read: a sweep stopped
            // before then didn't browse, rather than heard nothing.
            if browsed || searched {
                tx.send(Ev::Browsed {
                    dnssd: browsed,
                    ssdp: searched,
                });
            }
            let new = multicast_new(answered.iter().chain(upnp.keys()), &seen, &swept, lan, me);
            let here = |ip: &Ipv4Addr| {
                seen.contains(ip) && lan.is_some_and(|(at, bits)| same_subnet(at, *ip, bits))
            };
            let known: Vec<(Ipv4Addr, dnssd::Found)> = heard
                .iter()
                .filter(|(ip, _)| here(ip))
                .map(|(ip, f)| (*ip, f.clone()))
                .collect();
            if !known.is_empty() {
                tx.send(Ev::DnsSd(known));
            }
            let known: Vec<(Ipv4Addr, ssdp::Found)> = upnp
                .iter()
                .filter(|(ip, _)| here(ip))
                .map(|(ip, f)| (*ip, f.clone()))
                .collect();
            if !known.is_empty() {
                tx.send(Ev::Ssdp(known));
            }
            if !new.is_empty() && !tx.cancelled() {
                // A MAC only from an entry the OS confirmed, as for ARP.
                let table = mac::confirmed();
                let found: Vec<Heard> = new
                    .iter()
                    .map(|ip| {
                        (
                            *ip,
                            table.get(ip).copied(),
                            heard.remove(ip),
                            upnp.remove(ip),
                        )
                    })
                    .collect();
                net::par_for(&found, threads, rate, tx.cancel_flag(), |(ip, m, d, u)| {
                    let (name, name_from, own_mac) = if names {
                        identify(*ip, m.is_some(), ask)
                    } else {
                        (None, None, None)
                    };
                    let mac_from = match (m, own_mac) {
                        (Some(_), _) => Some("arp"),
                        (None, Some(_)) => Some("netbios"),
                        (None, None) => None,
                    };
                    tx.send(Ev::Alive(Host {
                        ip: *ip,
                        rtt: None,
                        ttl: None,
                        name,
                        name_from,
                        mac: m.or(own_mac),
                        mac_from,
                        open: Vec::new(),
                        checked: false,
                        also: Vec::new(),
                        web: None,
                        titled: false,
                        cert: None,
                        dnssd: d.clone().map(Box::new),
                        ssdp: u.clone().map(Box::new),
                        ais: None,
                        via: Via {
                            arp: m.is_some(),
                            mdns: d.is_some(),
                            ssdp: u.is_some(),
                            ..Via::default()
                        },
                    }));
                });
                for (ip, m, ..) in &found {
                    if let Some(m) = m {
                        macs.insert(*ip, *m);
                    }
                }
                seen.extend(new);
            }
            for (ip, ms) in conflicts(&readings.into_inner().unwrap(), &seen) {
                tx.send(Ev::Conflict(ip, ms));
            }

            if check_ports == PortCheck::Off || tx.cancelled() {
                return;
            }
            // Only now, with every host and its MAC known, pick the ones to
            // check. Hosts left out stay as the sweep found them.
            let hosts: Vec<(Ipv4Addr, Option<Mac>)> = seen
                .iter()
                .map(|&ip| (ip, macs.get(&ip).copied()))
                .collect();
            let targets = port_targets(check_ports, &hosts, gateway, me);
            let mut probes: Vec<(Ipv4Addr, u16)> = Vec::new();
            for &ip in &targets {
                probes.extend(ports::INFRA.iter().map(|&p| (ip, p)));
            }
            // Each host counts as checked once all its ports are: a stop
            // part way leaves the rest "not checked", not "none open".
            let left: Mutex<HashMap<Ipv4Addr, usize>> =
                Mutex::new(targets.iter().map(|&ip| (ip, ports::INFRA.len())).collect());
            tx.send(Ev::PortTargets(targets));
            let wait = Duration::from_millis(timeout as u64);
            let opened: Mutex<HashMap<Ipv4Addr, Vec<u16>>> = Mutex::new(HashMap::new());
            net::par_for(&probes, threads, rate, tx.cancel_flag(), |&(ip, port)| {
                let addr = SocketAddr::new(IpAddr::V4(ip), port);
                if net::tcp_check(addr, wait).0 == TcpState::Open {
                    tx.send(Ev::Open(ip, port));
                    opened.lock().unwrap().entry(ip).or_default().push(port);
                }
                tx.send(Ev::Probed);
                let done = left.lock().ok().and_then(|mut l| {
                    let n = l.get_mut(&ip)?;
                    *n -= 1;
                    (*n == 0).then_some(())
                });
                if done.is_some() {
                    tx.send(Ev::PortDone(ip));
                }
            });

            // What each device with a web page open calls itself: one GET
            // of its own page (two at most, and one redirect on itself).
            if tx.cancelled() {
                return;
            }
            let mut webby: Vec<(Ipv4Addr, Vec<u16>)> = opened
                .into_inner()
                .unwrap()
                .into_iter()
                .filter(|(_, ps)| ps.iter().any(|p| WEB_PORTS.contains(p)))
                .collect();
            webby.sort_unstable();
            tx.send(Ev::More(webby.len()));
            // A printer's or a switch's TLS takes a while.
            let page_wait = Duration::from_millis(timeout.max(2000) as u64);
            net::par_for(&webby, threads, rate, tx.cancel_flag(), |(ip, ps)| {
                let named = net::web::identify(IpAddr::V4(*ip), ps, page_wait);
                if named.label.is_some() || named.cert.is_some() {
                    tx.send(Ev::Web(*ip, Box::new(named)));
                }
                tx.send(Ev::Probed);
            });
        }));
    }
}

/// The inspector's rows for a device's own certificate: who it names,
/// its alternative names, who issued it and when it ends, with
/// self-signed and expired as plain notes (usual on a management page,
/// so never a finding).
fn cert_rows(ui: &mut Ui, c: &tls::Cert) {
    use crate::inspector as insp;
    if let Some(dn) = c.dn() {
        insp::row(ui, "Certificate", &dn, false);
    }
    if !c.names.is_empty() {
        insp::row(ui, "Cert names", &c.names.join(", "), true);
    }
    let issuer = if c.self_signed {
        "itself (self-signed)".to_string()
    } else {
        c.issuer.clone()
    };
    insp::row(ui, "Cert issued by", &issuer, false);
    if let Some(t) = c.not_after {
        let mut until = t.format("%Y-%m-%d").to_string();
        if c.expired(chrono::Utc::now()) {
            until += " (expired)";
        }
        insp::row(ui, "Cert valid until", &until, true);
    }
    insp::note(ui, CERT_NOTE);
}

const CERT_NOTE: &str = "From its HTTPS management page's handshake: the device's own claim, \
read and never trusted. A self-signed or expired certificate is usual on a device's own page.";

/// A report line's part for a device's certificate: its subject where the
/// page has no real title to name it, and the low notes (self-signed,
/// expired). Empty where there's nothing to add.
fn cert_words(c: &tls::Cert, titled: bool, now: chrono::DateTime<chrono::Utc>) -> String {
    let notes = c.notes(now);
    match c.dn().filter(|_| !titled) {
        Some(dn) if notes.is_empty() => format!("  cert \"{dn}\""),
        Some(dn) => format!("  cert \"{dn}\" ({})", notes.join(", ")),
        // Titled: only an expiry is worth a word.
        None => c
            .not_after
            .filter(|_| c.expired(now))
            .map(|t| format!("  cert expired {}", t.format("%Y-%m-%d")))
            .unwrap_or_default(),
    }
}

/// A live host's name and where it came from: DNS's (PTR), else the one
/// it gives for itself (NetBIOS, then multicast DNS), each waiting up to
/// `wait`. With no MAC from ARP (past a router) NetBIOS is asked even when
/// DNS has a name, for the MAC a Windows host reports, which comes last.
fn identify(
    ip: Ipv4Addr,
    have_mac: bool,
    wait: Duration,
) -> (Option<String>, Option<&'static str>, Option<Mac>) {
    let dns = net::reverse(IpAddr::V4(ip));
    if dns.is_some() && have_mac {
        return (dns, Some("dns"), None);
    }
    let nb = names::netbios(SocketAddr::new(IpAddr::V4(ip), names::NETBIOS_PORT), wait);
    let mac = nb.as_ref().and_then(|n| n.mac).filter(|_| !have_mac);
    if let Some(n) = dns {
        return (Some(n), Some("dns"), mac);
    }
    if let Some(n) = nb {
        return (Some(n.name), Some("netbios"), mac);
    }
    let md = names::mdns(ip, SocketAddr::new(IpAddr::V4(ip), names::MDNS_PORT), wait);
    let from = md.is_some().then_some("mdns");
    (md, from, mac)
}

/// A picked file read as an Advanced IP Scanner export: its name and what
/// it held, or why it won't read. Never more than `ais::MAX_FILE` is read.
fn read_import(p: &std::path::Path) -> Picked {
    use std::io::Read;
    let name = crate::tools::export::file_name(p);
    let meta = std::fs::metadata(p).map_err(|e| format!("{name}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{name}: not a regular file"));
    }
    let mut data = Vec::new();
    std::fs::File::open(p)
        .and_then(|f| f.take(ais::MAX_FILE + 1).read_to_end(&mut data))
        .map_err(|e| format!("{name}: {e}"))?;
    let scan = ais::parse(&data).map_err(|e| format!("{name}: {e}"))?;
    Ok((name, scan))
}

/// An alive row as a host. Its name is the one the scanner resolved, else
/// its NetBIOS name; its MAC is the scanner's.
fn imported_host(r: ais::Row) -> Option<Host> {
    let (name, name_from) = match (&r.name, &r.netbios_name) {
        (Some(n), _) => (Some(n.clone()), Some("ais")),
        (None, Some(n)) => (Some(n.clone()), Some("netbios")),
        (None, None) => (None, None),
    };
    Some(Host {
        ip: r.ip?,
        rtt: None,
        ttl: None,
        name,
        name_from,
        mac: r.mac,
        mac_from: r.mac.map(|_| "ais"),
        open: r.open.clone(),
        checked: r.checked,
        also: Vec::new(),
        web: r.web.clone(),
        titled: r.web.is_some(),
        cert: None,
        dnssd: None,
        ssdp: None,
        via: Via::AIS,
        ais: Some(Box::new(r)),
    })
}

/// "2 shares, 1 printer": counts only, the names are never read.
fn shared(a: &ais::Row) -> String {
    let n = |k: usize, one: &str| match k {
        0 => None,
        1 => Some(format!("1 {one}")),
        k => Some(format!("{k} {one}s")),
    };
    [n(a.shares, "share"), n(a.printers, "printer")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
}

const AIS_TIP: &str = "From an imported Advanced IP Scanner scan: it found this host alive. \
is-alive didn't ping it; Ping or Ports checks it from here.";

const AIS_NOTE: &str = "From an imported Advanced IP Scanner scan, as it found the host then. \
Logged-on users aren't imported, and shares and printers are counted, never named.";

const MULTICAST_TIP: &str = "Answered only the multicast browse (DNS-SD or SSDP), not ping or \
ARP. Listed because it's inside the swept range and on this computer's own segment, the only one \
multicast reaches.";

const MULTICAST_NOTE: &str = "Found only by the multicast browse (DNS-SD or SSDP): it didn't \
answer ping or ARP. Multicast stays on this computer's own segment, so only devices here can be \
found this way.";

const SSDP_NOTE: &str = "From SSDP (UPnP), the device's own claim. Its description URL is kept \
in the Inventory as text and never opened. Multicast stays on this computer's own segment.";

/// UPnP search targets as a person reads them: the device types
/// ("MediaRenderer", "InternetGatewayDevice", "dial"), not services.
fn upnp_kinds(types: &[String]) -> String {
    let mut v: Vec<&str> = types
        .iter()
        .filter_map(|t| {
            let mut p = t.split(':');
            (p.next()? == "urn").then_some(())?;
            p.next()?;
            (p.next()? == "device").then_some(())?;
            p.next().filter(|n| !n.is_empty())
        })
        .collect();
    v.dedup();
    v.join(", ")
}

/// Page languages as a person reads them: "pdf, vnd.hp-PCL, urf".
fn languages(pdl: &[String]) -> String {
    pdl.iter()
        .map(|l| l.split_once('/').map_or(l.as_str(), |(_, t)| t))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A host found only by the multicast browse: its address, a MAC from a
/// confirmed ARP entry, and what it said over DNS-SD and SSDP.
type Heard = (
    Ipv4Addr,
    Option<Mac>,
    Option<dnssd::Found>,
    Option<ssdp::Found>,
);

/// Of the addresses that answered the multicast browse, the ones to add
/// as hosts: not found yet, inside the swept range and on the scanner's
/// own segment (`lan`), and not this machine. Sorted.
fn multicast_new<'a>(
    heard: impl Iterator<Item = &'a Ipv4Addr>,
    seen: &HashSet<Ipv4Addr>,
    swept: &HashSet<Ipv4Addr>,
    lan: Option<(Ipv4Addr, u32)>,
    me: Option<Ipv4Addr>,
) -> Vec<Ipv4Addr> {
    let mut v: Vec<Ipv4Addr> = heard
        .copied()
        .filter(|ip| {
            !seen.contains(ip)
                && swept.contains(ip)
                && lan.is_some_and(|(at, bits)| same_subnet(at, *ip, bits))
                && Some(*ip) != me
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Under DNS-SD results: where they could come from.
const DNSSD_NOTE: &str = "From DNS-SD (Bonjour), the device's own claim. A computer or print \
server that shares a printer shows that printer's model. Multicast stays on this computer's own \
network segment: devices on routed networks can't answer it.";

/// What a host's right-click menu can copy, in order, each only when the
/// host has it: its address, name, MAC and vendor.
fn row_copies(h: &Host, vendor: &str) -> Vec<(&'static str, String)> {
    [
        ("IP", Some(h.ip.to_string())),
        ("name", h.name.clone()),
        ("MAC", h.mac.as_ref().map(mac::fmt)),
        ("vendor", Some(vendor.to_string())),
    ]
    .into_iter()
    .filter_map(|(w, v)| Some((w, v.filter(|v| !v.trim().is_empty())?)))
    .collect()
}

/// What Wake sends to for `h`: its MAC (a real card's), its address, and
/// the swept network holding it.
fn wake_target(h: &Host, nets: &[(Ipv4Addr, u32)]) -> Option<wol::Target> {
    let mac = h.mac.filter(wol::wakeable)?;
    Some(wol::Target {
        mac,
        ip: Some(h.ip),
        net: targets::network_of(nets, h.ip),
    })
}

/// Wake's tip: where the packet goes.
fn wake_tip(t: &wol::Target) -> String {
    let mut s = format!(
        "Send a Wake-on-LAN packet to {}, to wake it from sleep or shutdown",
        mac::fmt(&t.mac)
    );
    if let Some((net, bits)) = t.net {
        s.push_str(&format!(
            ". Off this computer's network it also goes to {net}/{bits}'s directed broadcast, \
             which only arrives if the router allows it (off by default on most)"
        ));
    }
    s
}

/// The table's columns.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Col {
    Address,
    Time,
    Ttl,
    Mac,
    Vendor,
    Name,
    Ports,
}

impl Col {
    fn title(self) -> &'static str {
        match self {
            Col::Address => "Address",
            Col::Time => "Time",
            Col::Ttl => "TTL",
            Col::Mac => "MAC",
            Col::Vendor => "Vendor",
            Col::Name => "Name",
            Col::Ports => "Open ports",
        }
    }

    /// Its width; Name takes what's left.
    fn width(self) -> f32 {
        match self {
            Col::Address => 120.0,
            Col::Time => 76.0,
            Col::Ttl => 44.0,
            Col::Mac => 156.0,
            Col::Vendor => 170.0,
            Col::Name => 160.0,
            Col::Ports => 150.0,
        }
    }
}

/// The columns shown, left to right.
struct Cols(Vec<Col>);

/// Room between columns, and at the row's ends.
const GAP: f32 = 8.0;

impl Cols {
    /// Each column's cell in the row `r`, left to right.
    fn cells(&self, r: Rect) -> Vec<(Col, Rect)> {
        let fixed: f32 = self
            .0
            .iter()
            .filter(|c| **c != Col::Name)
            .map(|c| c.width() + GAP)
            .sum();
        let name = (r.width() - fixed - 2.0 * GAP).max(Col::Name.width());
        let mut x = r.min.x + GAP;
        self.0
            .iter()
            .map(|&c| {
                let w = if c == Col::Name { name } else { c.width() };
                let cell = Rect::from_min_size(pos2(x, r.min.y), vec2(w, r.height()));
                x += w + GAP;
                (c, cell)
            })
            .collect()
    }
}

/// Every column that fits in `width`: Vendor goes first as it narrows, then
/// MAC (both are in the inspector). Open ports only after a port check; TTL
/// only when there is one (not for an imported scan).
fn columns(width: f32, ports: bool, ttl: bool) -> Cols {
    let mut all = vec![
        Col::Address,
        Col::Time,
        Col::Ttl,
        Col::Mac,
        Col::Vendor,
        Col::Name,
    ];
    if ports {
        all.push(Col::Ports);
    }
    if !ttl {
        all.retain(|c| *c != Col::Ttl);
    }
    let need = |cols: &[Col]| cols.iter().map(|c| c.width() + GAP).sum::<f32>() + GAP;
    // Vendor, then MAC (docs/dev/ui-layout.md), then TTL: at 620 wide with ports
    // checked, Ports would be cut off. The inspector shows all three.
    for drop in [Col::Vendor, Col::Mac, Col::Ttl] {
        if need(&all) > width {
            all.retain(|c| *c != drop);
        }
    }
    Cols(all)
}

/// `text` in the cell `r`, on one line, cut with … where it doesn't fit.
fn cell(p: &egui::Painter, r: Rect, text: &str, font: &FontId, colour: egui::Color32) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font.clone(), colour);
    job.wrap = egui::text::TextWrapping {
        max_width: r.width(),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let g = p.layout_job(job);
    p.galley(pos2(r.min.x, r.center().y - g.size().y / 2.0), g, colour);
}

fn found_by(h: &Host) -> Vec<&'static str> {
    h.via.list()
}

fn port_list(open: &[u16]) -> String {
    open.iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `a` and `b` are in the same /`bits` network.
fn same_subnet(a: Ipv4Addr, b: Ipv4Addr, bits: u32) -> bool {
    let mask = u32::MAX.checked_shl(32 - bits.min(32)).unwrap_or(0);
    u32::from(a) & mask == u32::from(b) & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(ip: [u8; 4], rtt: Option<f64>) -> Host {
        Host {
            ip: Ipv4Addr::from(ip),
            rtt,
            ttl: rtt.map(|_| 64),
            name: None,
            name_from: None,
            mac: None,
            mac_from: None,
            open: Vec::new(),
            checked: false,
            also: Vec::new(),
            web: None,
            titled: false,
            cert: None,
            dnssd: None,
            ssdp: None,
            ais: None,
            via: if rtt.is_some() { Via::PING } else { Via::ARP },
        }
    }

    fn mac(last: u8) -> Mac {
        [0x00, 0x11, 0x32, 0x00, 0x00, last]
    }

    /// A row's right-click copies only what the host has, address first.
    #[test]
    fn a_rows_menu_copies_only_what_the_host_has() {
        let bare = host([192, 0, 2, 20], None);
        let got = row_copies(&bare, "");
        assert_eq!(got, [("IP", "192.0.2.20".to_string())]);
        let mut full = host([192, 0, 2, 21], Some(1.0));
        full.name = Some("FRONT-DESK".into());
        full.mac = Some(mac(7));
        let got: Vec<&str> = row_copies(&full, "Synology")
            .into_iter()
            .map(|(w, _)| w)
            .collect();
        assert_eq!(got, ["IP", "name", "MAC", "vendor"]);
        assert_eq!(row_copies(&full, "  ")[2].1, mac::fmt(&mac(7)));
    }

    /// The Site Bundle's Clients setting reaches the inventory: a phone,
    /// laptop or PC's name (often a person's) becomes the UniFi export's
    /// token for its MAC, or goes; the gateway, gear and NAS keep theirs.
    #[test]
    fn inventory_names_follow_the_clients_setting() {
        let named = |ip: [u8; 4], m: Option<Mac>, name: &str| {
            let mut h = host(ip, Some(1.0));
            (h.name, h.name_from) = (Some(name.into()), Some("netbios"));
            (h.mac, h.mac_from) = (m, m.map(|_| "arp"));
            h
        };
        let intel: Mac = [0x00, 0x12, 0xF0, 0, 0, 9];
        let phone: Mac = [0x02, 0, 0, 0, 0, 8];
        let mut s = swept(vec![
            named([10, 0, 0, 1], Some(intel), "EDGE-GW"),
            named([10, 0, 0, 20], Some(mac(2)), "BACKUP-NAS"),
            named([10, 0, 0, 30], Some(intel), "DR-SMITH-PC"),
            named([10, 0, 0, 31], Some(phone), "Janes-iPhone"),
            named([10, 0, 0, 32], None, "front-desk"),
        ]);
        s.local.gateway = Some(Ipv4Addr::new(10, 0, 0, 1));
        let names = |c: Clients| {
            let v: serde_json::Value =
                serde_json::from_str(&s.inventory(&HostInfo::default(), c)).unwrap();
            assert_eq!(v["client_names"], c.label());
            v["hosts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| {
                    assert_eq!(h["name_from"], "netbios", "still says where it came from");
                    h["name"].clone()
                })
                .collect::<Vec<_>>()
        };
        let full = names(Clients::Full);
        assert_eq!(full[2], "DR-SMITH-PC");
        let anon = names(Clients::Anonymized);
        assert_eq!(anon[0], "EDGE-GW", "the gateway");
        assert_eq!(anon[1], "BACKUP-NAS", "a Synology: a NAS");
        assert_eq!(anon[2], json!(crate::unifi::token(&mac::fmt(&intel))));
        assert_eq!(anon[3], json!(crate::unifi::token("02:00:00:00:00:08")));
        assert_eq!(
            anon[4],
            json!(crate::unifi::token("10.0.0.32")),
            "no MAC: by address"
        );
        let none = names(Clients::None);
        assert_eq!(
            (&none[1], &none[2], &none[3]),
            (&json!("BACKUP-NAS"), &json!(null), &json!(null))
        );
    }

    fn swept(hosts: Vec<Host>) -> SweepTool {
        let mut s = SweepTool {
            range: "10.0.0.0/24".into(),
            ran: "10.0.0.0/24".into(),
            total: 254,
            ..Default::default()
        };
        s.apply(hosts.into_iter().map(Ev::Alive).collect());
        s
    }

    #[test]
    fn vendor_then_mac_drop_out_as_the_table_narrows() {
        let has = |w: f32, c: Col| columns(w, false, true).0.contains(&c);
        // Room for everything.
        assert!(has(900.0, Col::Vendor) && has(900.0, Col::Mac));
        // Vendor goes first.
        assert!(!has(700.0, Col::Vendor) && has(700.0, Col::Mac));
        // Then MAC; the address, time and name always stay.
        for c in [Col::Address, Col::Time, Col::Name] {
            assert!(has(400.0, c));
        }
        assert!(!has(400.0, Col::Mac));
        // Open ports only after a port check, and it keeps its place last.
        assert!(!has(1400.0, Col::Ports));
        assert_eq!(columns(1400.0, true, true).0.last(), Some(&Col::Ports));
        // No TTL to show (an imported scan): no column for it.
        assert!(!columns(1400.0, true, false).0.contains(&Col::Ttl));
        // At 620 wide (about 548 for the table), with ports checked, TTL
        // goes too so Ports isn't cut off.
        let narrow = columns(548.0, true, true);
        let fits = narrow.0.iter().map(|c| c.width() + GAP).sum::<f32>() + GAP;
        assert!(fits <= 548.0, "{fits}");
        assert!(narrow.0.contains(&Col::Ports) && !narrow.0.contains(&Col::Ttl));
    }

    /// One frame of the Sweep tab in a window `size` big: every text drawn,
    /// and what the inspector asked for.
    fn sweep_frame(
        ctx: &egui::Context,
        s: &mut SweepTool,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> (Vec<(String, Rect)>, Option<crate::inspector::Ask>) {
        fn texts(sh: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match sh {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_string(), t.visual_bounding_rect()));
                }
                egui::Shape::Vec(v) => v.iter().for_each(|x| texts(x, out)),
                _ => {}
            }
        }
        let input = egui::RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
            time: Some(ctx.input(|i| i.time) + 0.1),
            ..Default::default()
        };
        let info = HostInfo::default();
        let mut ask = None;
        let mut out = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| ask = s.ui(ui, &info));
        });
        out.textures_delta.clear();
        let mut found = Vec::new();
        for c in &out.shapes {
            texts(&c.shape, &mut found);
        }
        (found, ask)
    }

    fn click_at(
        ctx: &egui::Context,
        s: &mut SweepTool,
        size: egui::Vec2,
        at: egui::Pos2,
    ) -> (Vec<(String, Rect)>, Option<crate::inspector::Ask>) {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        sweep_frame(ctx, s, size, vec![egui::Event::PointerMoved(at)]);
        sweep_frame(ctx, s, size, vec![press(true)]);
        let (_, ask) = sweep_frame(ctx, s, size, vec![press(false)]);
        let (t, later) = sweep_frame(ctx, s, size, vec![]);
        (t, ask.or(later))
    }

    fn at(t: &[(String, Rect)], label: &str) -> egui::Pos2 {
        t.iter()
            .find(|(s, _)| s == label)
            .unwrap_or_else(|| panic!("no {label:?}"))
            .1
            .center()
    }

    /// On a laptop-sized window a click on a host opens it in the
    /// inspector, whose buttons ask for the tests and the Map; another
    /// click on the row closes it.
    #[test]
    fn a_host_opens_in_the_inspector_with_its_tests() {
        for size in [vec2(1280.0, 720.0), vec2(1000.0, 700.0)] {
            let ctx = egui::Context::default();
            let mut h = host([10, 0, 0, 7], Some(1.5));
            h.name = Some("printer-2f".into());
            h.name_from = Some("mdns");
            h.mac = Some(mac(7));
            let mut s = swept(vec![host([10, 0, 0, 1], Some(0.4)), h]);
            s.detected = true;
            let (t, _) = sweep_frame(&ctx, &mut s, size, vec![]);
            let (t, _) = click_at(&ctx, &mut s, size, at(&t, "printer-2f"));
            assert_eq!(s.picked, Some(Ipv4Addr::new(10, 0, 0, 7)), "{size:?}");
            for label in [
                "Ping",
                "Ports",
                "Trace",
                "Web",
                "On map",
                "the device, over multicast DNS",
            ] {
                at(&t, label);
            }
            let (_, ask) = click_at(&ctx, &mut s, size, at(&t, "Ports"));
            assert_eq!(ask, Some(crate::inspector::Ask::Ports("10.0.0.7".into())));
            let (t, ask) = click_at(&ctx, &mut s, size, at(&t, "On map"));
            assert_eq!(
                ask,
                Some(crate::inspector::Ask::OnMap(Ipv4Addr::new(10, 0, 0, 7)))
            );
            // The row again: closed.
            // The row's name, not the inspector's heading: the leftmost.
            let row = t
                .iter()
                .filter(|(x, _)| x == "printer-2f")
                .min_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x))
                .expect("the row")
                .1
                .center();
            click_at(&ctx, &mut s, size, row);
            assert_eq!(s.picked, None, "{size:?}");
        }
    }

    #[test]
    fn hosts_stay_in_address_order_and_take_their_mac_and_ports() {
        let mut s = SweepTool::default();
        let changed = s.apply(vec![
            Ev::Alive(host([10, 0, 0, 20], Some(1.0))),
            Ev::Alive(host([10, 0, 0, 3], None)),
            Ev::Probed,
            Ev::Mac(Ipv4Addr::new(10, 0, 0, 20), mac(1)),
            Ev::Open(Ipv4Addr::new(10, 0, 0, 20), 443),
            Ev::Open(Ipv4Addr::new(10, 0, 0, 20), 22),
            Ev::PortTargets(vec![Ipv4Addr::new(10, 0, 0, 20)]),
            Ev::PortDone(Ipv4Addr::new(10, 0, 0, 20)),
        ]);
        assert!(changed);
        let ips: Vec<String> = s.alive.iter().map(|h| h.ip.to_string()).collect();
        assert_eq!(ips, ["10.0.0.3", "10.0.0.20"]);
        assert_eq!(s.alive[1].mac, Some(mac(1)));
        assert_eq!(s.alive[1].open, [22, 443]);
        assert_eq!((s.alive[0].checked, s.alive[1].checked), (false, true));
        assert_eq!((s.probed, s.total), (1, ports::INFRA.len()));
    }

    #[test]
    fn a_mac_or_port_for_a_host_not_found_changes_nothing() {
        let mut s = swept(vec![host([10, 0, 0, 1], Some(0.5))]);
        let changed = s.apply(vec![
            Ev::Mac(Ipv4Addr::new(10, 0, 0, 99), mac(9)),
            Ev::Open(Ipv4Addr::new(10, 0, 0, 99), 80),
            Ev::Probed,
        ]);
        assert!(!changed, "the map shouldn't rebuild for nothing");
        assert_eq!(s.alive[0].mac, None);
        assert!(s.alive[0].open.is_empty());
    }

    #[test]
    fn report_counts_arp_only_hosts_and_lists_ports() {
        let mut a = host([10, 0, 0, 1], Some(0.42));
        a.open = vec![22, 443];
        a.name = Some("gw.lan".into());
        let s = swept(vec![a, host([10, 0, 0, 7], None)]);
        let r = s.report();
        assert!(
            r.starts_with("Sweep of 10.0.0.0/24: 2 alive (1 found by ARP only, no ping reply)\n")
        );
        assert!(r.contains("10.0.0.1") && r.contains("0.4 ms") && r.contains("gw.lan  [22 443]"));
        assert!(r.contains("ARP only"));
        assert!(r.lines().all(|l| l == l.trim_end()), "no trailing spaces");
        let t = s.table().unwrap();
        assert_eq!(t.rows[0][8], "ping");
        assert_eq!(t.rows[1][8], "arp");
        assert_eq!(t.rows[0][7], "22 443");
    }

    /// A typo in the box and Enter: the last sweep stays, whole. And what
    /// a finished sweep says it swept is what it swept, whatever the box
    /// says later.
    #[test]
    fn a_typo_keeps_the_last_sweep_and_reports_name_what_ran() {
        let mut s = swept(vec![
            host([10, 0, 0, 1], Some(1.0)),
            host([10, 0, 0, 9], Some(2.0)),
        ]);
        let before = (s.alive.len(), s.total, s.revision, s.runs);
        s.range = "10.0.1.0/2x".into();
        s.start(&egui::Context::default());
        assert!(s.err.is_some() && s.job.is_none());
        assert_eq!((s.alive.len(), s.total, s.revision, s.runs), before);
        // The next network typed in, not yet swept.
        s.range = "192.168.9.0/24".into();
        assert!(s.report().starts_with("Sweep of 10.0.0.0/24"));
        assert_eq!(s.finished(), Some(("10.0.0.0/24".to_string(), 2)));
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        assert_eq!(
            (&v["range"], &v["ranges"]),
            (&json!("10.0.0.0/24"), &json!(["10.0.0.0/24"]))
        );
        assert!(
            s.table()
                .unwrap()
                .rows
                .iter()
                .all(|r| r[0] == "10.0.0.0/24")
        );
    }

    /// The gateway answering on a virtual MAC (a SonicWall's LAN: its own
    /// prefix with the locally-administered bit set) is named by that
    /// maker and never called private; a phone's private MAC still is.
    #[test]
    fn a_gateway_on_a_virtual_mac_is_not_a_private_client() {
        let mut gw = host([192, 0, 2, 1], Some(1.0));
        (gw.mac, gw.mac_from) = (Some([0x2e, 0xb8, 0xed, 0x0a, 0x0b, 0x0c]), Some("arp"));
        (gw.name, gw.name_from) = (Some("fw-lab".into()), Some("dns"));
        let mut phone = host([192, 0, 2, 40], Some(3.0));
        (phone.mac, phone.mac_from) = (Some([0x2e, 0xb8, 0xed, 0x44, 0x55, 0x66]), Some("arp"));
        (phone.name, phone.name_from) = (Some("Sams-Phone".into()), Some("mdns"));
        let mut s = swept(vec![gw, phone, host([192, 0, 2, 50], Some(1.0))]);
        s.ran = "192.0.2.0/24".into();
        s.local = HostInfo {
            lan: Some((Ipv4Addr::new(192, 0, 2, 50), 24)),
            gateway: Some(Ipv4Addr::new(192, 0, 2, 1)),
            ..Default::default()
        };
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Anonymized)).unwrap();
        let h = &v["hosts"][0];
        assert_eq!(h["is_gateway"], true);
        assert_eq!(
            (&h["vendor"], &h["private_mac"], &h["virtual_mac"]),
            (&json!("SonicWall"), &json!(false), &json!(true))
        );
        assert_eq!(h["name"], "fw-lab", "the gateway keeps its name");
        assert_eq!(s.vendor(&s.alive[0]), "SonicWall (virtual MAC)");
        // Same maker's prefix, but no port or page of its own: private.
        let p = &v["hosts"][1];
        assert_eq!(
            (&p["vendor"], &p["private_mac"], &p["virtual_mac"]),
            (&json!(null), &json!(true), &json!(false))
        );
        assert_ne!(p["name"], "Sams-Phone");
        // A laptop whose random MAC clears to a Cisco prefix, answering
        // 445 and 3389: no gear of its own, so private, name anonymized.
        let mut laptop = host([192, 0, 2, 41], Some(2.0));
        (laptop.mac, laptop.mac_from) = (Some([0x02, 0x00, 0x0c, 0x01, 0x02, 0x03]), Some("arp"));
        (laptop.name, laptop.name_from) = (Some("Jos-Laptop".into()), Some("netbios"));
        (laptop.open, laptop.checked) = (vec![445, 3389], true);
        assert_eq!(
            crate::net::mac::universal_vendor(&laptop.mac.unwrap()),
            Some("Cisco Systems")
        );
        let mut s2 = swept(vec![laptop]);
        s2.local = s.local.clone();
        let v2: serde_json::Value =
            serde_json::from_str(&s2.inventory(&HostInfo::default(), Clients::Anonymized)).unwrap();
        let l = &v2["hosts"][0];
        assert_eq!(
            (&l["vendor"], &l["private_mac"], &l["virtual_mac"]),
            (&json!(null), &json!(true), &json!(false))
        );
        assert_ne!(l["name"], "Jos-Laptop");
        // No MAC: neither.
        assert!(v["hosts"][2]["private_mac"].is_null() && v["hosts"][2]["virtual_mac"].is_null());
        let notes = v["notes"].to_string();
        assert!(
            notes.contains("virtual_mac true") && notes.contains("Poly"),
            "{notes}"
        );
    }

    #[test]
    fn inventory_marks_the_gateway_and_this_machine_and_ports_only_if_checked() {
        let mut gw = host([10, 0, 0, 1], Some(1.0));
        gw.open = vec![443];
        let mut s = swept(vec![gw, host([10, 0, 0, 50], None)]);
        s.local = HostInfo {
            lan: Some((Ipv4Addr::new(10, 0, 0, 50), 24)),
            gateway: Some(Ipv4Addr::new(10, 0, 0, 1)),
            ..Default::default()
        };
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        let hosts = v["hosts"].as_array().unwrap();
        assert_eq!(hosts[0]["is_gateway"], true);
        assert_eq!(hosts[1]["is_this_machine"], true);
        assert_eq!(hosts[1]["found_by"], json!(["arp"]));
        assert_eq!(hosts[0]["found_by"], json!(["ping"]));
        assert!(hosts[1]["rtt_ms"].is_null());
        assert!(hosts[0]["open_ports"].is_null(), "ports weren't checked");
        assert_eq!(v["scanner"]["lan"], "10.0.0.50/24");
        assert_eq!(v["range"], "10.0.0.0/24");
        assert_eq!(v["ranges"], json!(["10.0.0.0/24"]));
        assert_eq!(hosts[0]["on_scanner_lan"], true);

        // Several networks: a routed host is marked so its missing MAC
        // isn't read as an unknown device.
        s.ran = "10.0.0.0/24, 172.20.5.0 - 20".into();
        s.alive.push(host([172, 20, 5, 9], Some(4.0)));
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        assert_eq!(v["ranges"], json!(["10.0.0.0/24", "172.20.5.0-20"]));
        assert_eq!(v["hosts"][2]["on_scanner_lan"], false);
        assert!(v["hosts"][2]["mac"].is_null() && v["hosts"][2]["mac_from"].is_null());
        // A Windows host past the router that reported its own name and MAC.
        let mut win = host([172, 20, 5, 10], Some(4.0));
        (win.name, win.name_from) = (Some("FRONTDESK-PC".into()), Some("netbios"));
        (win.mac, win.mac_from) = (Some(mac(7)), Some("netbios"));
        s.alive.push(win);
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        let w = &v["hosts"][3];
        assert_eq!(
            (&w["name"], &w["name_from"], &w["mac"], &w["mac_from"]),
            (
                &json!("FRONTDESK-PC"),
                &json!("netbios"),
                &json!("00:11:32:00:00:07"),
                &json!("netbios")
            )
        );
        assert_eq!(w["on_scanner_lan"], false);

        // Every field a host carries is one the bundled /mapnetwork skill
        // explains: a new one has to be described there before it ships.
        let skill = include_str!("../../skills/mapnetwork/SKILL.md");
        for key in w.as_object().unwrap().keys() {
            assert!(
                skill.contains(&format!("`{key}`")),
                "skills/mapnetwork/SKILL.md doesn't explain the inventory field {key}"
            );
        }
        assert!(same_subnet(
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 0, 0, 254),
            24
        ));
        assert!(!same_subnet(
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 0, 1, 1),
            24
        ));
        assert!(same_subnet(
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(99, 0, 0, 1),
            0
        ));

        // A Smart check: the gateway checked (443 open), the scanner not
        // (null: not checked), a routed host checked with nothing open ([]).
        s.ran_ports = PortCheck::Smart;
        s.alive[0].checked = true;
        s.alive[2].checked = true;
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        assert_eq!(v["hosts"][0]["open_ports"][0]["port"], 443);
        assert!(v["hosts"][1]["open_ports"].is_null());
        assert_eq!(v["hosts"][2]["open_ports"], json!([]));
        assert_eq!(
            v["ports_checked"].as_array().unwrap().len(),
            ports::INFRA.len()
        );
    }

    /// A printer's DNS-SD answer reaches the report and the inventory, which
    /// say the browse only covers the scanner's own segment.
    #[test]
    fn dnssd_models_reach_the_report_and_the_inventory() {
        let mut s = swept(vec![
            host([10, 0, 0, 40], Some(1.0)),
            host([10, 0, 0, 41], Some(1.0)),
        ]);
        s.apply(vec![
            Ev::Browsed {
                dnssd: true,
                ssdp: false,
            },
            Ev::DnsSd(vec![(
                Ipv4Addr::new(10, 0, 0, 40),
                dnssd::Found {
                    services: vec!["_ipp._tcp".into(), "_pdl-datastream._tcp".into()],
                    ty: Some("HP LaserJet Pro M404".into()),
                    product: None,
                    rp: vec!["ipp/print".into()],
                    pdl: vec!["application/pdf".into()],
                },
            )]),
        ]);
        let r = s.report();
        assert!(
            r.contains(
                "DNS-SD browse: 1 answered; 0 found only this way (this computer's own segment only"
            ),
            "{r}"
        );
        assert!(r.contains("model \"HP LaserJet Pro M404\""), "{r}");
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        assert_eq!(v["dnssd_browsed"], true);
        assert_eq!(v["hosts"][0]["dnssd"]["ty"], "HP LaserJet Pro M404");
        assert_eq!(
            v["hosts"][0]["dnssd"]["services"][1],
            "_pdl-datastream._tcp"
        );
        assert!(v["hosts"][0]["dnssd"]["product"].is_null());
        assert_eq!(v["hosts"][0]["dnssd"]["rp"], json!(["ipp/print"]));
        assert_eq!(v["hosts"][0]["dnssd"]["pdl"], json!(["application/pdf"]));
        assert_eq!(v["hosts"][0]["found_by"], json!(["ping", "mdns"]));
        assert!(v["hosts"][1]["dnssd"].is_null());
        assert_eq!(v["ssdp_searched"], false);
        let notes = v["notes"].to_string();
        assert!(notes.contains("own segment"), "{notes}");
        // Every top-level key is one the skill names.
        let skill = include_str!("../../skills/mapnetwork/SKILL.md");
        assert!(skill.contains("`dnssd_browsed`") && skill.contains("`ty`"));
        for k in ["rp", "pdl", "ssdp_searched"] {
            assert!(skill.contains(&format!("`{k}`")), "{k}");
        }
        // A new sweep forgets it.
        s.clear_results();
        assert!(!s.browsed);
    }

    /// Wake goes to a real card's MAC, with the swept network for its
    /// directed broadcast; a private MAC or none gets no Wake.
    #[test]
    fn wake_targets_carry_the_swept_network() {
        let mut a = host([10, 20, 0, 5], Some(1.0));
        a.mac = Some(mac(5));
        let mut b = host([10, 20, 0, 6], Some(1.0));
        b.mac = Some([0xDA, 0x11, 0x32, 0, 0, 6]);
        let mut s = swept(vec![a, b, host([10, 20, 0, 7], Some(1.0))]);
        s.ran = "10.20.0.0/24".into();
        let w = s.wakeable();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].1.ip, Some(Ipv4Addr::new(10, 20, 0, 5)));
        assert_eq!(w[0].1.net, Some((Ipv4Addr::new(10, 20, 0, 0), 24)));
        assert!(wake_tip(&w[0].1).contains("directed broadcast"));
        s.ran = "10.20.0.1-50".into();
        assert_eq!(s.wakeable()[0].1.net, None);
    }

    /// Smart checks the gateway, gear, NAS, printers, cameras and makers it
    /// doesn't know; not a private MAC, a laptop or phone network card's
    /// maker, or this machine. Off checks nothing, All everything.
    #[test]
    fn smart_checks_likely_gear_and_unknowns_not_phones_and_laptops() {
        use crate::map::ports_worth_checking as worth;
        for v in [
            "Ubiquiti Inc",
            "Synology Incorporated",
            "Brother industries",
            "Hikvision",
        ] {
            assert!(worth(Some(v), false), "{v}");
        }
        for v in [
            "Intel Corporate",
            "Apple",
            "Samsung Electronics",
            "REALTEK SEMICONDUCTOR",
        ] {
            assert!(!worth(Some(v), false), "{v}");
        }
        assert!(
            worth(Some("Shenzhen Something Tech"), false),
            "an unknown maker"
        );
        assert!(worth(None, false), "no vendor: ports are the only clue");
        assert!(!worth(None, true), "a private MAC is a phone or laptop");
        assert!(
            worth(Some("Quanta Network Systems"), false),
            "switches, not laptops"
        );

        let at = |last: u8| Ipv4Addr::new(10, 0, 0, last);
        let m = |oui: [u8; 3]| Some([oui[0], oui[1], oui[2], 0x00, 0x00, 0x01]);
        let hosts = [
            (at(1), m([0x00, 0x12, 0xF0])),  // the gateway, though Intel's
            (at(2), m([0x00, 0x15, 0x6D])),  // Ubiquiti
            (at(3), m([0x00, 0x12, 0xF0])),  // Intel: a laptop
            (at(4), m([0x00, 0x03, 0x93])),  // Apple
            (at(5), m([0x02, 0x11, 0x22])),  // private MAC
            (at(6), None),                   // routed: no MAC
            (at(7), m([0x00, 0x11, 0x32])),  // Synology
            (at(50), m([0x00, 0x15, 0x6D])), // this machine
        ];
        let smart = port_targets(PortCheck::Smart, &hosts, Some(at(1)), Some(at(50)));
        assert_eq!(smart, [at(1), at(2), at(6), at(7)]);
        assert!(port_targets(PortCheck::Off, &hosts, Some(at(1)), Some(at(50))).is_empty());
        assert_eq!(
            port_targets(PortCheck::All, &hosts, Some(at(1)), Some(at(50))).len(),
            hosts.len()
        );
        assert_eq!(SweepTool::default().ports, PortCheck::Smart);
    }

    /// An address that answered from two MACs (each a fresh or confirmed
    /// ARP entry) is flagged on its host, in the report, the inventory and
    /// the map; one MAC, or a MAC for a host not found, flags nothing.
    #[test]
    fn an_address_answering_from_two_macs_is_a_duplicate_ip() {
        let ip = |last: u8| Ipv4Addr::new(10, 0, 0, last);
        let readings: HashMap<Ipv4Addr, std::collections::BTreeSet<Mac>> = [
            (ip(5), [mac(5)].into()),
            (ip(7), [mac(7), mac(9)].into()),
            (ip(99), [mac(1), mac(2)].into()), // not a live host
        ]
        .into();
        let live: HashSet<Ipv4Addr> = [ip(5), ip(7)].into();
        assert_eq!(conflicts(&readings, &live), [(ip(7), vec![mac(7), mac(9)])]);

        let mut h = host([10, 0, 0, 7], Some(1.0));
        h.mac = Some(mac(7));
        let mut s = swept(vec![host([10, 0, 0, 5], Some(1.0)), h]);
        s.apply(vec![Ev::Conflict(ip(7), vec![mac(7), mac(9)])]);
        assert_eq!(s.alive[1].also, [mac(9)], "the other one, not its own");
        assert!(s.alive[0].also.is_empty());
        let r = s.report();
        assert!(
            r.contains("1 address(es) answered from more than one MAC"),
            "{r}"
        );
        assert!(r.contains("also from 00:11:32:00:00:09"), "{r}");
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        assert_eq!(v["hosts"][1]["other_macs"], json!(["00:11:32:00:00:09"]));
        assert_eq!(v["hosts"][0]["other_macs"], json!([]));
        assert_eq!(s.seen()[1].also, [mac(9)]);

        // A web page's title rides along the same way.
        let page = net::web::Named {
            label: Some("UniFi OS".into()),
            titled: true,
            cert: None,
        };
        s.apply(vec![Ev::More(1), Ev::Web(ip(5), Box::new(page))]);
        assert_eq!(s.alive[0].web.as_deref(), Some("UniFi OS"));
        assert!(s.report().contains("\"UniFi OS\""));
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        assert_eq!(v["hosts"][0]["web_title"], "UniFi OS");
        assert!(v["hosts"][1]["web_title"].is_null());
        assert_eq!(s.seen()[0].web.as_deref(), Some("UniFi OS"));
    }

    /// A host on 169.254.x.x is flagged in the report with the DHCP hint;
    /// FTP is among the ports checked.
    #[test]
    fn a_self_assigned_host_is_flagged_in_the_report() {
        let s = swept(vec![
            host([192, 0, 2, 5], Some(1.0)),
            host([169, 254, 3, 4], Some(1.0)),
        ]);
        let r = s.report();
        assert!(
            r.contains(&format!(
                "⚠ 1 self-assigned address(es) (169.254.x.x): {}",
                crate::map::NO_DHCP
            )),
            "{r}"
        );
        let line = r.lines().find(|l| l.starts_with("169.254.3.4")).unwrap();
        assert!(line.contains("⚠ self-assigned, no DHCP"), "{line}");
        let other = r.lines().find(|l| l.starts_with("192.0.2.5")).unwrap();
        assert!(!other.contains("self-assigned"));
        assert!(
            !swept(vec![host([192, 0, 2, 5], Some(1.0))])
                .report()
                .contains("self-assigned")
        );
        assert!(ports::INFRA.contains(&21));
    }

    /// A device's certificate rides along with its page: shown where the
    /// title was a placeholder, in the inventory's tls object, on the Map,
    /// and as a role hint; self-signed and expired are notes only.
    #[test]
    fn a_devices_certificate_reaches_the_report_inventory_and_map() {
        let ip = |last: u8| Ipv4Addr::new(10, 0, 0, last);
        let mut fw = host([10, 0, 0, 1], Some(1.0));
        (fw.open, fw.checked) = (vec![443], true);
        let mut nas = host([10, 0, 0, 2], Some(1.0));
        (nas.open, nas.checked) = (vec![443], true);
        let mut s = swept(vec![fw, nas]);
        let at = |y, m, d| {
            chrono::NaiveDate::from_ymd_opt(y, m, d)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
        };
        let fgt = tls::Cert {
            subject: "FGT60FTK00000000".into(),
            cn: Some("FGT60FTK00000000".into()),
            unit: Some("FortiGate".into()),
            org: Some("Fortinet".into()),
            issuer: "FGT60FTK00000000".into(),
            names: vec!["fw.branch.example".into()],
            not_after: Some(at(2038, 1, 19)),
            self_signed: true,
            ..Default::default()
        };
        let nas_cert = tls::Cert {
            cn: Some("DS-0000".into()),
            issuer: "Acme Test CA".into(),
            not_after: Some(at(2024, 3, 1)),
            ..Default::default()
        };
        s.apply(vec![
            Ev::More(2),
            // "Loading..." was dropped: the Server header stands in.
            Ev::Web(
                ip(1),
                Box::new(net::web::Named {
                    label: Some("xxxxxxxx-xxxxx".into()),
                    titled: false,
                    cert: Some(fgt),
                }),
            ),
            Ev::Web(
                ip(2),
                Box::new(net::web::Named {
                    label: Some("Synology DiskStation".into()),
                    titled: true,
                    cert: Some(nas_cert),
                }),
            ),
        ]);
        let r = s.report();
        assert!(
            r.contains("cert \"CN=FGT60FTK00000000, OU=FortiGate, O=Fortinet\" (self-signed)"),
            "{r}"
        );
        // A real title names it: only the expiry is worth a word.
        assert!(r.contains("cert expired 2024-03-01"), "{r}");
        assert!(!r.contains("DS-0000"), "{r}");

        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        let t = &v["hosts"][0]["tls"];
        assert_eq!(
            t["subject"],
            "CN=FGT60FTK00000000, OU=FortiGate, O=Fortinet"
        );
        assert_eq!(t["sans"], json!(["fw.branch.example"]));
        assert_eq!(t["issuer"], "FGT60FTK00000000");
        assert_eq!(t["not_after"], "2038-01-19T00:00:00Z");
        assert_eq!(t["self_signed"], true);
        assert_eq!(v["hosts"][1]["tls"]["subject"], "CN=DS-0000");
        assert_eq!(v["hosts"][1]["tls"]["self_signed"], false);
        let notes = v["notes"].to_string();
        assert!(notes.contains("tls is the certificate"), "{notes}");
        // Each tls key is one the bundled skill explains.
        let skill = include_str!("../../skills/mapnetwork/SKILL.md");
        for key in t.as_object().unwrap().keys() {
            assert!(skill.contains(&format!("`{key}`")), "SKILL.md: {key}");
        }

        // The Map gets it, and it names the firewall's role.
        let seen = s.seen();
        assert!(!seen[0].titled && seen[1].titled);
        assert_eq!(
            seen[0].cert.as_ref().and_then(tls::Cert::dn).as_deref(),
            Some("CN=FGT60FTK00000000, OU=FortiGate, O=Fortinet")
        );
        let words = seen[0].cert.as_ref().map(tls::Cert::words);
        let said = crate::map::Said {
            page: seen[0].web.as_deref(),
            dnssd: None,
            cert: words.as_deref(),
        };
        assert!(!crate::map::looks_like_client(None, &[443], said));

        // A later answer with a certificate but no page keeps the label.
        s.apply(vec![Ev::Web(
            ip(2),
            Box::new(net::web::Named {
                label: None,
                titled: false,
                cert: None,
            }),
        )]);
        assert_eq!(s.alive[1].web.as_deref(), Some("Synology DiskStation"));
        assert!(s.alive[1].titled && s.alive[1].cert.is_some());
    }

    /// The port pass only adds: every host the sweep found stays, with what
    /// it found, whether it was checked or not, and when the run stops.
    #[test]
    fn every_host_the_sweep_found_survives_the_port_pass() {
        let named = |last: u8, name: &str| {
            let mut h = host([10, 0, 0, last], Some(1.0));
            (h.name, h.name_from) = (Some(name.into()), Some("dns"));
            (h.mac, h.mac_from) = (Some(mac(last)), Some("arp"));
            h
        };
        let mut s = SweepTool {
            ran_ports: PortCheck::Smart,
            ..Default::default()
        };
        s.apply(vec![
            Ev::Alive(named(1, "gw")),
            Ev::Alive(named(30, "laptop")),
            Ev::Alive(named(40, "nas")),
        ]);
        s.apply(vec![
            Ev::PortTargets(vec![
                Ipv4Addr::new(10, 0, 0, 1),
                Ipv4Addr::new(10, 0, 0, 40),
            ]),
            Ev::Open(Ipv4Addr::new(10, 0, 0, 1), 443),
            // The gateway's ports all tried; the NAS's not, when it stopped.
            Ev::PortDone(Ipv4Addr::new(10, 0, 0, 1)),
        ]);
        s.job = None;
        s.stopped = true;
        let got: Vec<(String, Option<&str>, bool, Vec<u16>)> = s
            .alive
            .iter()
            .map(|h| {
                (
                    h.ip.to_string(),
                    h.name.as_deref(),
                    h.checked,
                    h.open.clone(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("10.0.0.1".to_string(), Some("gw"), true, vec![443]),
                ("10.0.0.30".to_string(), Some("laptop"), false, vec![]),
                // Not finished: "not checked", never "none open".
                ("10.0.0.40".to_string(), Some("nas"), false, vec![]),
            ]
        );
        assert!(s.alive.iter().all(|h| h.mac.is_some()));
        assert_eq!(s.seen().len(), 3, "the map gets all three");
        let r = s.report();
        assert!(r.contains("Mgmt ports checked on 1 of 3"), "{r}");
        assert!(r.contains("Stopped before it finished"), "{r}");
        let inv: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        assert_eq!(inv["complete"], false);
        assert!(
            inv["hosts"][2]["open_ports"].is_null(),
            "the NAS wasn't finished"
        );
        let none = SweepTool {
            ran_ports: PortCheck::Smart,
            ..Default::default()
        };
        assert!(
            !none.report().contains("Mgmt ports"),
            "nothing alive, nothing to say"
        );
        assert!(
            ["10.0.0.1", "10.0.0.30", "10.0.0.40"]
                .iter()
                .all(|ip| r.contains(ip))
        );
    }

    /// A device silent to ping and ARP that answered the multicast
    /// browse is added, but only inside the swept range, on the
    /// scanner's own segment, and not this machine.
    #[test]
    fn only_in_range_on_segment_multicast_answers_become_hosts() {
        let ip = |l: u8| Ipv4Addr::new(192, 0, 2, l);
        let heard = [
            ip(40),
            ip(75),
            ip(75),
            ip(99),
            ip(50),
            Ipv4Addr::new(198, 51, 100, 7),
        ];
        let seen: HashSet<Ipv4Addr> = [ip(40)].into();
        // The sweep covered .1-.80 of the scanner's /24 (and the routed
        // 198.51.100.0/24, which multicast can't reach).
        let mut swept: HashSet<Ipv4Addr> = (1..=80).map(ip).collect();
        swept.insert(Ipv4Addr::new(198, 51, 100, 7));
        let lan = Some((ip(50), 24));
        assert_eq!(
            multicast_new(heard.iter(), &seen, &swept, lan, Some(ip(50))),
            [ip(75)]
        );
        // No known segment: nothing is added.
        assert!(multicast_new(heard.iter(), &seen, &swept, None, None).is_empty());
    }

    /// A multicast-only host says how it was found everywhere: the report,
    /// the CSV, the inventory and the Map.
    #[test]
    fn a_multicast_only_host_says_how_it_was_found() {
        let mut s = swept(vec![host([10, 0, 0, 40], Some(1.0))]);
        s.local = HostInfo {
            lan: Some((Ipv4Addr::new(10, 0, 0, 50), 24)),
            ..Default::default()
        };
        let mut tv = host([10, 0, 0, 75], None);
        tv.via = Via {
            mdns: true,
            ssdp: true,
            ..Via::default()
        };
        tv.ssdp = Some(Box::new(ssdp::Found {
            server: Some("Linux/4.9 UPnP/1.0 ExampleCast/1.0".into()),
            types: vec!["urn:schemas-upnp-org:device:MediaRenderer:1".into()],
            uuids: vec!["uuid:00000000-0000-4000-8000-000000000075".into()],
            locations: vec!["http://10.0.0.75:8008/desc.xml".into()],
        }));
        s.apply(vec![
            Ev::Browsed {
                dnssd: true,
                ssdp: true,
            },
            Ev::Alive(tv),
            Ev::Ssdp(vec![(
                Ipv4Addr::new(10, 0, 0, 40),
                ssdp::Found {
                    server: Some("HP HTTP Server".into()),
                    ..Default::default()
                },
            )]),
        ]);
        assert_eq!(s.arp_only(), 0);
        assert_eq!(s.multicast_only(), 1);
        let r = s.report();
        assert!(r.contains("1 found by mDNS or SSDP only"), "{r}");
        assert!(
            r.contains(
                "SSDP search: 2 answered; 1 found only this way (this computer's own segment"
            ),
            "{r}"
        );
        assert!(
            r.contains("mDNS+SSDP") && r.contains("(found by mdns, ssdp)"),
            "{r}"
        );
        assert!(r.contains("(found by ping, ssdp)"), "{r}");
        let t = s.table().unwrap();
        assert_eq!(t.rows[1].last().unwrap(), "mdns ssdp");
        let v: serde_json::Value =
            serde_json::from_str(&s.inventory(&HostInfo::default(), Clients::Full)).unwrap();
        let h = &v["hosts"][1];
        assert_eq!(h["found_by"], json!(["mdns", "ssdp"]));
        assert!(h["rtt_ms"].is_null());
        assert_eq!(h["on_scanner_lan"], true);
        assert_eq!(
            h["ssdp"]["st"][0],
            "urn:schemas-upnp-org:device:MediaRenderer:1"
        );
        assert_eq!(v["ssdp_searched"], true);
        let notes = v["notes"].to_string();
        assert!(notes.contains("never fetches"), "{notes}");
        // Every field of the ssdp and dnssd records is one the skill names.
        let skill = include_str!("../../skills/mapnetwork/SKILL.md");
        for key in h["ssdp"].as_object().unwrap().keys() {
            assert!(skill.contains(&format!("`{key}`")), "ssdp.{key}");
        }
        for key in v["hosts"][0].as_object().unwrap().keys() {
            assert!(skill.contains(&format!("`{key}`")), "{key}");
        }
        let seen = s.seen();
        assert_eq!(seen[1].multicast.as_deref(), Some("mDNS and SSDP"));
        assert_eq!(seen[0].multicast, None);
        assert_eq!(upnp_kinds(&h_types()), "MediaRenderer, dial");
        assert_eq!(
            languages(&["application/pdf".into(), "image/urf".into()]),
            "pdf, urf"
        );
    }

    /// An imported Advanced IP Scanner export: its alive rows are the
    /// hosts, the rest counted, and it reaches the report, the CSV, the
    /// inventory and the Map like a sweep. Who was logged on, the share
    /// and printer names, and the file's name never get in.
    #[test]
    fn an_imported_ais_scan_flows_like_a_sweep_without_users_or_share_names() {
        let mut s = SweepTool::default();
        s.load_import("acme-scan.xml", ais::parse(ais::SAMPLE.as_bytes()).unwrap());
        assert_eq!(s.alive.len(), 3);
        assert_eq!(s.finished(), Some(("192.0.2.0/24".to_string(), 3)));
        assert!(s.checked_ports(), "its service checks count");
        let r = s.report();
        assert!(
            r.starts_with(
                "Advanced IP Scanner scan of 192.0.2.0/24 (2026-11-24 13:05:12 UTC-05:00), \
                 imported: 3 alive\n"
            ),
            "{r}"
        );
        assert!(r.contains("Not imported: 1 dead and 2 unknown"), "{r}");
        for want in [
            "OS \"Windows 11 Pro\"",
            "RDP \"RDP 10.0\"",
            "2 shares, 1 printer",
            "TLSv1.2",
            "workgroup EXAMPLE",
            "NetBIOS FRONTDESK-PC",
            "(found by ais)",
            "imported",
        ] {
            assert!(r.contains(want), "{want}: {r}");
        }
        assert!(r.lines().all(|l| l == l.trim_end()), "no trailing spaces");
        let t = s.table().unwrap();
        assert_eq!(t.rows[1][8], "ais");
        assert_eq!(t.rows[1][6], "frontdesk.example.lan");
        assert_eq!(t.rows[2][6], "NAS01", "no DNS name: its NetBIOS one");
        assert_eq!(t.rows[2][7], "21 8080");
        assert_eq!(t.rows[0][5], "Example Networks", "the scanner's maker");

        let inv = |c: Clients| -> serde_json::Value {
            serde_json::from_str(&s.inventory(&HostInfo::default(), c)).unwrap()
        };
        let v = inv(Clients::Full);
        assert_eq!(v["scanned_at"], "2026-11-24 13:05:12 UTC-05:00");
        assert_eq!(
            (&v["imported"]["dead"], &v["imported"]["unknown"]),
            (&json!(1), &json!(2))
        );
        assert_eq!(v["ports_checked"], json!([21, 80, 443, 3389, 8080]));
        assert!(v["scanner"]["lan"].is_null() && v["scanner"]["wan"].is_null());
        assert!(
            v["source"]
                .as_str()
                .unwrap()
                .contains("Advanced IP Scanner")
        );
        let pc = &v["hosts"][1];
        assert_eq!(pc["found_by"], json!(["ais"]));
        assert_eq!(
            (&pc["mac_from"], &pc["name_from"]),
            (&json!("ais"), &json!("ais"))
        );
        assert!(pc["rtt_ms"].is_null() && pc["on_scanner_lan"].is_null());
        assert_eq!(pc["open_ports"][0]["port"], 3389);
        assert_eq!(pc["ais"]["netbios_name"], "FRONTDESK-PC");
        assert_eq!(pc["ais"]["os"], "Windows 11 Pro");
        assert_eq!(
            (&pc["ais"]["shares"], &pc["ais"]["printers"]),
            (&json!(2), &json!(1))
        );
        assert_eq!(v["hosts"][0]["ais"]["tls"], "TLSv1.2");
        assert_eq!(v["hosts"][0]["vendor"], "Example Networks");
        assert_eq!(v["hosts"][2]["name_from"], "netbios");
        // A sweep's own hosts say null.
        let swept_v: serde_json::Value = serde_json::from_str(
            &swept(vec![host([10, 0, 0, 1], Some(1.0))])
                .inventory(&HostInfo::default(), Clients::Full),
        )
        .unwrap();
        assert!(swept_v["imported"].is_null() && swept_v["hosts"][0]["ais"].is_null());

        // Every field it adds is one the bundled skill explains.
        let skill = include_str!("../../skills/mapnetwork/SKILL.md");
        let keys = |x: &serde_json::Value| -> Vec<String> {
            x.as_object().unwrap().keys().cloned().collect()
        };
        for k in keys(pc).into_iter().chain(keys(&pc["ais"])) {
            assert!(
                skill.contains(&format!("`{k}`")),
                "SKILL.md doesn't explain {k}"
            );
        }
        assert!(skill.contains("`imported`"));
        for k in keys(&v["imported"]) {
            assert!(
                skill.contains(&format!("`imported.{k}`")),
                "SKILL.md: imported.{k}"
            );
        }
        let notes = v["notes"].to_string();
        assert!(
            notes.contains("Logged-on users are never imported"),
            "{notes}"
        );

        // Nothing private, anywhere it goes.
        let mut out = vec![r, format!("{:?}", t.rows), format!("{:?}", s.seen())];
        for c in [Clients::Full, Clients::Anonymized, Clients::None] {
            out.push(s.inventory(&HostInfo::default(), c));
        }
        for o in &out {
            for never in [
                "jdoe",
                "ghost",
                "Payroll",
                "Scans",
                "Front Desk HP",
                "acme-scan",
            ] {
                assert!(!o.contains(never), "{never} in {o}");
            }
        }

        // On the Map: an imported line, not "ARP only".
        let seen = s.seen();
        let line = seen[1].imported.as_deref().unwrap();
        assert!(line.starts_with(crate::map::IMPORTED), "{line}");
        assert!(
            line.contains("2026-11-24") && line.contains("Windows 11 Pro"),
            "{line}"
        );
        let m = crate::map::build(&crate::map::Input {
            sweep: seen,
            range: s.finished().map(|f| f.0),
            ports_checked: true,
            ..Default::default()
        });
        let card = m
            .nodes
            .iter()
            .find(|n| n.ip == Some(Ipv4Addr::new(192, 0, 2, 20)))
            .unwrap();
        assert!(
            card.details
                .iter()
                .any(|d| d.starts_with(crate::map::IMPORTED)),
            "{:?}",
            card.details
        );
        assert!(
            !card.chips.iter().any(|c| c == "ARP only"),
            "{:?}",
            card.chips
        );

        // The HTTPS banner is its own row, never the web page's title.
        assert_eq!(s.alive[0].web, None);
        assert_eq!(v["hosts"][0]["ais"]["https_server"], "Web-Server httpd 3.0");
        // The scanner's network is unknown: null, not an object of nulls.
        assert!(v["scanner"].is_null());
        assert!(!swept_v["scanner"].is_null());

        // A new sweep, or Clear map, forgets it was an import.
        s.clear_results();
        assert!(s.imported.is_none() && s.alive.is_empty());
    }

    /// An import still being picked or read never lands after Clear map
    /// or a new sweep: it would cancel the sweep and lose its Results
    /// entry, or (after a site switch, which waits for it) fill the
    /// wrong site.
    #[test]
    fn an_import_in_progress_is_dropped_by_clear_and_start() {
        let ctx = egui::Context::default();
        let pending = |ctx: &egui::Context| {
            Some(Job::spawn(ctx, |tx: crate::job::Tx<Picked>| {
                tx.sleep(Duration::from_millis(50));
                tx.send(Ok((
                    "late.xml".into(),
                    ais::parse(ais::SAMPLE.as_bytes()).unwrap(),
                )));
            }))
        };
        let mut s = swept(vec![host([10, 0, 0, 1], Some(1.0))]);
        s.importing = pending(&ctx);
        assert!(s.importing());
        s.clear_results();
        assert!(!s.importing());
        // Start drops it, even with a range that won't read (so nothing
        // is sent from a test).
        s.importing = pending(&ctx);
        s.range = "10.0.0.1/2x".into();
        s.start(&ctx);
        assert!(!s.importing() && s.job.is_none());
        std::thread::sleep(Duration::from_millis(150));
        s.poll();
        assert!(!s.imported(), "the late file never landed");
    }

    /// A PC's NetBIOS name is often a person's: it follows the Site
    /// Bundle's Clients setting like the host's name.
    #[test]
    fn an_imported_pcs_netbios_name_follows_the_clients_setting() {
        let rows = "<row status='alive' ip='192.0.2.30' mac='00:12:F0:00:00:09' name='' \
                    netbiosname='DR-SMITH-PC' netbiosgroup='EXAMPLE' user='EXAMPLE\\drsmith'/>";
        let mut s = SweepTool::default();
        s.load_import("scan.xml", ais::parse(rows.as_bytes()).unwrap());
        let host = |c: Clients| -> serde_json::Value {
            let v: serde_json::Value =
                serde_json::from_str(&s.inventory(&HostInfo::default(), c)).unwrap();
            v["hosts"][0].clone()
        };
        let full = host(Clients::Full);
        assert_eq!(
            (&full["name"], &full["ais"]["netbios_name"]),
            (&json!("DR-SMITH-PC"), &json!("DR-SMITH-PC"))
        );
        let token = json!(crate::unifi::token("00:12:f0:00:00:09"));
        let anon = host(Clients::Anonymized);
        assert_eq!(
            (&anon["name"], &anon["ais"]["netbios_name"]),
            (&token, &token)
        );
        let none = host(Clients::None);
        assert!(none["name"].is_null() && none["ais"]["netbios_name"].is_null());
        assert_eq!(
            none["ais"]["netbios_group"], "EXAMPLE",
            "a workgroup isn't a person"
        );
        for c in [Clients::Full, Clients::Anonymized, Clients::None] {
            assert!(!s.inventory(&HostInfo::default(), c).contains("drsmith"));
        }
    }

    /// The tab after an import: the hosts, how the scan was had, and the
    /// PC's details in the inspector, at the two laptop sizes.
    #[test]
    fn an_imported_scan_shows_on_the_tab_and_in_the_inspector() {
        for size in [vec2(1280.0, 720.0), vec2(1120.0, 720.0), vec2(620.0, 400.0)] {
            let ctx = egui::Context::default();
            let mut s = SweepTool::default();
            s.sample_import_for_shots();
            sweep_frame(&ctx, &mut s, size, vec![]);
            let (t, _) = sweep_frame(&ctx, &mut s, size, vec![]);
            for label in [
                "Import scan…",
                "· imported · 3 not alive",
                "HOST · IMPORTED",
                "Windows 11 Pro",
                "FRONTDESK-PC",
                "2 shares, 1 printer (names not imported)",
            ] {
                // At 620 × 400 the inspector scrolls: its top only.
                if size.y > 500.0 || label.starts_with(['I', '·', 'H']) {
                    at(&t, label);
                }
            }
            assert!(
                !t.iter()
                    .any(|(x, _)| x.contains("jdoe") || x.contains("Payroll"))
            );
            // A real scan's hundreds of dead and unknown rows still leave
            // the buttons room.
            if let Some(im) = s.imported.as_mut() {
                (im.dead, im.unknown) = (120, 377);
            }
            let (t, _) = sweep_frame(&ctx, &mut s, size, vec![]);
            let rect = |label: &str| t.iter().find(|(x, _)| x == label).unwrap().1;
            let (said, button) = (rect("· imported · 497 not alive"), rect("Import scan…"));
            assert!(said.max.x < button.min.x, "{size:?}: {said:?} {button:?}");
        }
    }

    fn h_types() -> Vec<String> {
        vec![
            "upnp:rootdevice".into(),
            "urn:schemas-upnp-org:device:MediaRenderer:1".into(),
            "urn:schemas-upnp-org:service:AVTransport:1".into(),
            "urn:dial-multiscreen-org:device:dial:1".into(),
        ]
    }
}
