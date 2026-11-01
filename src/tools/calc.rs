//! Subnet Calc tab: type a block, mask or range and see what it holds,
//! check whether an address is inside, split it into smaller blocks, or get
//! the fewest CIDR blocks that cover a range. Nothing leaves the machine.

use crate::net::hostinfo::HostInfo;
use crate::net::subnet::{self, Block, Info, Input};
use crate::net::targets;
use crate::widgets;
use eframe::egui::{self, RichText, Ui};
use std::net::IpAddr;

/// Split rows at most: 2^16 (a /16 into /32s, a /48 into /64s). The table
/// draws only the rows on screen, so this is about keeping copies sane.
const SPLIT_MAX_BITS: u32 = 16;

pub struct CalcTool {
    input: String,
    detected: bool,
    check: String,
    /// Split into blocks of this prefix length.
    split: u8,
    /// The parsed input, kept while the text is unchanged.
    parsed: Option<(String, Result<Input, String>)>,
}

impl Default for CalcTool {
    fn default() -> Self {
        Self {
            input: "192.168.1.0/24".into(),
            detected: false,
            check: String::new(),
            split: 0,
            parsed: None,
        }
    }
}

impl CalcTool {
    /// Returns a range to sweep when **Sweep** is clicked.
    pub fn ui(&mut self, ui: &mut Ui, info: &HostInfo, sweeping: bool) -> Option<String> {
        // First time the tab is shown: start from this machine's subnet.
        if !self.detected {
            self.detected = true;
            if let Some((ip, bits)) = info.lan {
                self.input = format!("{ip}/{bits}");
            }
        }
        if self.parsed.as_ref().is_none_or(|(s, _)| *s != self.input) {
            self.parsed = Some((self.input.clone(), subnet::parse(&self.input)));
        }
        let parsed = self
            .parsed
            .as_ref()
            .map(|(_, p)| p.clone())
            .expect("set above");
        let mut sweep = None;

        ui.horizontal(|ui| {
            widgets::field(
                ui,
                "Subnet",
                &mut self.input,
                300.0,
                "10.0.0.0/22, 10.0.0.5 255.255.252.0, 2001:db8::/48 or 10.0.0.1-99",
            );
            if let Some((ip, bits)) = info.lan
                && ui
                    .small_button("This network")
                    .on_hover_text(format!("{ip}/{bits}"))
                    .clicked()
            {
                self.input = format!("{ip}/{bits}");
            }
        });
        widgets::toolbar_note(
            ui,
            "A prefix (/22), a netmask (255.255.252.0) or an ACL wildcard (0.0.3.255) after \
             the address; IPv6 too. A range shows the CIDR blocks that cover it.",
        );
        ui.separator();
        widgets::status_row(
            ui,
            |_| {},
            |ui| {
                let ok = parsed.is_ok();
                ui.add_enabled_ui(ok, |ui| {
                    widgets::save_button(ui, "subnet", || self.report(&parsed));
                    widgets::copy_button(ui, || self.report(&parsed));
                });
                let target = parsed.as_ref().ok().and_then(sweepable);
                let tip = match (&parsed, &target) {
                    (_, Some(_)) if sweeping => "Subnet Sweep is already running".to_string(),
                    (_, Some(_)) => "Ping-sweep these addresses in Subnet Sweep".to_string(),
                    (Ok(_), None) => format!(
                        "Subnet Sweep takes IPv4 only, up to {} addresses",
                        subnet::group(targets::MAX_HOSTS as u128)
                    ),
                    (Err(_), None) => "Enter a subnet or range first".into(),
                };
                if ui
                    .add_enabled(target.is_some() && !sweeping, egui::Button::new("Sweep"))
                    .on_hover_text(&tip)
                    .on_disabled_hover_text(&tip)
                    .clicked()
                {
                    sweep = target;
                }
            },
        );

        let input = match parsed {
            Ok(i) => i,
            Err(e) => {
                if self.input.trim().is_empty() {
                    widgets::empty_state(
                        ui,
                        "Work out a subnet",
                        "Enter a subnet, an address and mask, or a range.",
                        &[],
                    );
                } else {
                    widgets::error_text(ui, &e);
                }
                return sweep;
            }
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| match input {
                Input::Block(b) => self.block_ui(ui, b),
                Input::Range(a, z) => range_ui(ui, a, z),
            });
        sweep
    }

    fn block_ui(&mut self, ui: &mut Ui, b: Block) {
        let i = b.info();
        details(ui, &b, &i);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            widgets::field(
                ui,
                "Is this address inside?",
                &mut self.check,
                220.0,
                if b.v6() { "2001:db8::1" } else { "10.0.3.7" },
            );
            let c = self.check.trim();
            if !c.is_empty() {
                match c.parse::<IpAddr>() {
                    Ok(ip) => match b.offset(ip) {
                        Some(n) => ui.label(
                            RichText::new(format!("✔ Inside {} (address #{n})", b.cidr()))
                                .color(widgets::ok()),
                        ),
                        None => ui.label(
                            RichText::new(format!("✖ Not in {}", b.cidr())).color(widgets::bad()),
                        ),
                    },
                    Err(_) => ui.label(RichText::new("not an IP address").weak()),
                };
            }
        });

        ui.add_space(8.0);
        ui.separator();
        let width = if b.v6() { 128 } else { 32 };
        let lo = b.prefix.saturating_add(1).min(width);
        let hi = (u32::from(b.prefix) + SPLIT_MAX_BITS).min(u32::from(width)) as u8;
        if b.prefix >= width {
            ui.weak("A single address: nothing to split.");
            return;
        }
        if !(lo..=hi).contains(&self.split) {
            // A sensible first pick: halves, or /24s and /64s from larger blocks.
            let nice = if b.v6() { 64 } else { 24 };
            self.split = if (lo..=hi).contains(&nice) { nice } else { lo };
        }
        let n_bits = b.split_bits(self.split);
        let n = 1u128 << n_bits;
        let mut copy = false;
        ui.horizontal(|ui| {
            ui.strong("Split into");
            ui.add(
                egui::DragValue::new(&mut self.split)
                    .range(lo..=hi)
                    .prefix("/"),
            );
            let each = b.sub(self.split, 0).info();
            let size = subnet::count(each.host_bits, 1u128.checked_shl(each.host_bits));
            ui.label(if b.v6() {
                format!("{} blocks of {size} addresses", subnet::group(n))
            } else {
                format!(
                    "{} blocks of {size} addresses ({} usable each)",
                    subnet::group(n),
                    subnet::count(each.host_bits, each.usable),
                )
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                copy = ui
                    .button("Copy list")
                    .on_hover_text("Copy every block, one per line")
                    .clicked();
            });
        });
        if copy {
            let text: String = (0..n).map(|k| b.sub(self.split, k).cidr() + "\n").collect();
            widgets::copy(ui.ctx(), text);
            widgets::toast("Copied to the clipboard", true);
        }
        let v6 = b.v6();
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + ui.spacing().item_spacing.y;
        let heads: &[&str] = if v6 {
            &["#", "Block", "First", "Last"]
        } else {
            &["#", "Block", "First host", "Last host", "Broadcast"]
        };
        ui.horizontal(|ui| {
            for (h, w) in heads.iter().zip(cols(v6)) {
                cell(ui, w, row_h, RichText::new(*h).strong());
            }
        });
        // Rows are drawn only when on screen, so 65,536 of them cost nothing.
        let rows = usize::try_from(n).unwrap_or(usize::MAX);
        egui::ScrollArea::vertical()
            .id_salt("split_rows")
            .max_height(360.0)
            .auto_shrink([false, true])
            .show_rows(ui, row_h, rows, |ui, range| {
                for k in range {
                    let s = b.sub(self.split, k as u128).info();
                    ui.horizontal(|ui| {
                        let mut cells = vec![
                            subnet::group(k as u128 + 1),
                            format!("{}/{}", s.network, s.prefix),
                            s.first_host.to_string(),
                            s.last_host.to_string(),
                        ];
                        if !v6 {
                            cells.push(s.last.to_string());
                        }
                        for (c, w) in cells.into_iter().zip(cols(v6)) {
                            cell(ui, w, row_h, RichText::new(c).monospace());
                        }
                    });
                }
            });
    }

    fn report(&self, parsed: &Result<Input, String>) -> String {
        match parsed {
            Ok(Input::Block(b)) => {
                let i = b.info();
                let mut s: String = rows(b, &i)
                    .into_iter()
                    .map(|(k, v)| format!("{k:<18}{v}\n"))
                    .collect();
                if let Ok(ip) = self.check.trim().parse::<IpAddr>() {
                    s += &match b.offset(ip) {
                        Some(n) => format!("{:<18}{ip} is inside (address #{n})\n", "Check"),
                        None => format!("{:<18}{ip} is not inside\n", "Check"),
                    };
                }
                s
            }
            Ok(Input::Range(a, z)) => {
                let blocks = subnet::cover(*a, *z);
                let mut s = format!(
                    "Range {a} - {z}: {} addresses in {} CIDR block{}\n",
                    subnet::range_len(*a, *z),
                    blocks.len(),
                    if blocks.len() == 1 { "" } else { "s" }
                );
                for b in blocks {
                    s += &format!("{}\n", b.cidr());
                }
                s
            }
            Err(e) => e.clone(),
        }
    }
}

/// Column widths for the split table: number, block, then addresses.
const COL: [f32; 3] = [56.0, 170.0, 130.0];

fn cols(v6: bool) -> Vec<f32> {
    if v6 {
        vec![COL[0], 300.0, 300.0, 300.0]
    } else {
        vec![COL[0], COL[1], COL[2], COL[2], COL[2]]
    }
}

/// A left-aligned table cell `w` wide.
fn cell(ui: &mut Ui, w: f32, h: f32, text: RichText) {
    ui.allocate_ui_with_layout(
        egui::vec2(w, h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(w);
            ui.label(text);
        },
    );
}

/// (label, value) for a block, as shown and as copied.
fn rows(b: &Block, i: &Info) -> Vec<(&'static str, String)> {
    let v6 = b.v6();
    let total = 1u128.checked_shl(i.host_bits);
    let mut r = vec![
        ("Address", i.ip.to_string()),
        ("Block", b.cidr()),
        ("Type", i.kind.clone()),
        ("Netmask", format!("{} (/{})", i.mask, i.prefix)),
    ];
    if !v6 {
        r.push(("Wildcard", i.wildcard.to_string()));
        if let Some(bin) = subnet::binary(i.mask) {
            r.push(("Mask in binary", bin));
        }
    }
    r.push(("Network", i.network.to_string()));
    if v6 {
        r.push(("Last address", i.last.to_string()));
    } else {
        r.push(("Broadcast", i.last.to_string()));
        r.push(("First host", i.first_host.to_string()));
        r.push(("Last host", i.last_host.to_string()));
    }
    r.push(("Addresses", subnet::count(i.host_bits, total)));
    if !v6 {
        let note = match i.prefix {
            31 => " (point-to-point link, RFC 3021)",
            32 => " (a single host)",
            _ => "",
        };
        r.push(("Usable hosts", subnet::count(i.host_bits, i.usable) + note));
    } else if i.prefix <= 64 {
        r.push((
            "/64 networks",
            subnet::count(
                64 - u32::from(i.prefix),
                1u128.checked_shl(64 - u32::from(i.prefix)),
            ),
        ));
    }
    let neighbour =
        |ip: Option<IpAddr>| ip.map_or_else(|| "none".into(), |ip| format!("{ip}/{}", i.prefix));
    r.push(("Previous block", neighbour(i.prev)));
    r.push(("Next block", neighbour(i.next)));
    r
}

fn details(ui: &mut Ui, b: &Block, i: &Info) {
    egui::Grid::new("calc")
        .num_columns(2)
        .striped(true)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            for (k, v) in rows(b, i) {
                ui.label(k);
                let text = RichText::new(&v).monospace();
                let text = if k == "Block" { text.strong() } else { text };
                if ui
                    .add(egui::Label::new(text).sense(egui::Sense::click()))
                    .on_hover_text("Click to copy")
                    .clicked()
                {
                    widgets::copy(ui.ctx(), v.clone());
                    widgets::toast(format!("Copied {v}"), true);
                }
                ui.end_row();
            }
        });
}

fn range_ui(ui: &mut Ui, a: IpAddr, z: IpAddr) {
    let blocks = subnet::cover(a, z);
    ui.label(format!(
        "{a} to {z}: {} addresses, covered exactly by {} CIDR block{}",
        subnet::range_len(a, z),
        blocks.len(),
        if blocks.len() == 1 { "" } else { "s" }
    ));
    ui.weak(
        "The fewest blocks that hold exactly this range, as a firewall address group needs them.",
    );
    ui.add_space(4.0);
    egui::Grid::new("cover").striped(true).show(ui, |ui| {
        ui.strong("Block");
        ui.strong("First");
        ui.strong("Last");
        ui.strong("Addresses");
        ui.end_row();
        for b in blocks {
            let i = b.info();
            ui.monospace(b.cidr());
            ui.monospace(i.network.to_string());
            ui.monospace(i.last.to_string());
            ui.monospace(subnet::count(i.host_bits, 1u128.checked_shl(i.host_bits)));
            ui.end_row();
        }
    });
}

/// What Subnet Sweep can take from this input, if anything: IPv4, and no
/// more addresses than it sweeps.
fn sweepable(input: &Input) -> Option<String> {
    let (spec, n) = match input {
        Input::Block(b) if !b.v6() => (b.cidr(), 1u64 << (32 - u32::from(b.prefix))),
        Input::Range(IpAddr::V4(a), IpAddr::V4(z)) => (
            format!("{a}-{z}"),
            u64::from(u32::from(*z) - u32::from(*a)) + 1,
        ),
        _ => return None,
    };
    // A block's network and broadcast addresses aren't swept.
    let swept = if matches!(input, Input::Block(b) if b.prefix <= 30) {
        n - 2
    } else {
        n
    };
    (swept <= targets::MAX_HOSTS as u64).then_some(spec)
}
