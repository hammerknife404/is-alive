//! Connect buttons on a Map card: SSH and Remote Desktop to the card's
//! address, in the system's own clients. Nothing is stored: the client asks
//! for the user name and password, as it would started by hand.
//!
//! Safe by construction: every program is found by absolute path in the
//! system's folders (never on PATH), and the one thing taken from the map is
//! the address, an `Ipv4Addr` that prints as digits and dots. The rest of
//! each command line is fixed text.

use std::net::Ipv4Addr;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Ssh,
    Rdp,
}

impl Kind {
    /// The port that must be open on the card for the button to show.
    pub fn port(self) -> u16 {
        match self {
            Kind::Ssh => 22,
            Kind::Rdp => 3389,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Ssh => "SSH",
            Kind::Rdp => "RDP",
        }
    }
}

/// What a button runs: a program by absolute path, its arguments, and the
/// words for the hover text ("ssh 10.0.0.1 in Konsole").
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    program: PathBuf,
    args: Vec<String>,
    /// Windows only: the arguments as one string, for cmd.exe's own parsing.
    raw: Option<String>,
    pub what: String,
}

/// The plan for `kind`, or None when this computer has no client for it
/// (the button is hidden then).
pub fn plan(kind: Kind, ip: Ipv4Addr) -> Option<Plan> {
    clients().get(kind).map(|c| c.plan(ip))
}

/// Start it, off the UI thread. A toast says so if it can't.
pub fn launch(plan: Plan) {
    std::thread::spawn(move || {
        if let Err(e) = run(&plan) {
            crate::widgets::toast(format!("Couldn't start {} ({e})", plan.what), false);
        }
    });
}

/// The clients found on this computer, looked for once.
#[derive(Default)]
struct Clients {
    ssh: Option<Client>,
    rdp: Option<Client>,
}

impl Clients {
    fn get(&self, kind: Kind) -> Option<&Client> {
        match kind {
            Kind::Ssh => self.ssh.as_ref(),
            Kind::Rdp => self.rdp.as_ref(),
        }
    }
}

fn clients() -> &'static Clients {
    static C: std::sync::OnceLock<Clients> = std::sync::OnceLock::new();
    C.get_or_init(find)
}

/// How a client is started.
#[derive(Clone, Debug, PartialEq)]
enum Client {
    /// Windows: ssh.exe in a new console that stays open after it exits,
    /// so a refused connection can be read. Windows 11 hands the console
    /// to Windows Terminal when that's the default terminal.
    #[cfg_attr(not(windows), allow(dead_code))]
    WinSsh { cmd: PathBuf, ssh: PathBuf },
    /// Windows: Remote Desktop Connection.
    #[cfg_attr(not(windows), allow(dead_code))]
    Mstsc(PathBuf),
    /// Linux: `client …ip` in a terminal emulator, held open after it exits.
    #[cfg_attr(windows, allow(dead_code))]
    InTerminal {
        term: Terminal,
        client: PathBuf,
        /// Fixed arguments before the address, and how the address is
        /// written ("{}" or "/v:{}").
        pre: &'static [&'static str],
        addr: &'static str,
    },
    /// Linux: a client with its own window and log-in dialog.
    #[cfg_attr(windows, allow(dead_code))]
    Direct {
        client: PathBuf,
        pre: &'static [&'static str],
        addr: &'static str,
        name: &'static str,
    },
}

/// A terminal emulator and how it's told to run a command.
#[derive(Clone, Debug, PartialEq)]
struct Terminal {
    path: PathBuf,
    name: &'static str,
    /// Arguments before the command.
    run: &'static [&'static str],
}

/// Shown when a run in the terminal ends, until Enter: a refused
/// connection stays on screen. The client and address arrive as $0 and $@,
/// never in the script's text.
const HOLD: &str =
    "\"$0\" \"$@\"; printf '\\n[%s ended: press Enter to close]' \"${0##*/}\"; read -r _";

impl Client {
    fn plan(&self, ip: Ipv4Addr) -> Plan {
        let ip = ip.to_string();
        let addr = |pattern: &str| pattern.replace("{}", &ip);
        match self {
            Client::WinSsh { cmd, ssh } => Plan {
                program: cmd.clone(),
                args: Vec::new(),
                // /s /k "<cmd line>": cmd strips the outer quotes only.
                raw: Some(format!("/s /k \"\"{}\" {ip}\"", ssh.display())),
                what: format!("ssh {ip} in a new console window"),
            },
            Client::Mstsc(p) => Plan {
                program: p.clone(),
                args: vec![format!("/v:{ip}")],
                raw: None,
                what: format!("Remote Desktop to {ip}"),
            },
            Client::InTerminal {
                term,
                client,
                pre,
                addr: a,
            } => {
                let mut args: Vec<String> = term.run.iter().map(|s| s.to_string()).collect();
                args.extend(["/bin/sh".into(), "-c".into(), HOLD.into()]);
                args.push(client.display().to_string());
                args.extend(pre.iter().map(|s| s.to_string()));
                args.push(addr(a));
                let short = client.file_name().map_or_else(
                    || client.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                Plan {
                    program: term.path.clone(),
                    args,
                    raw: None,
                    what: format!("{short} {ip} in {}", term.name),
                }
            }
            Client::Direct {
                client,
                pre,
                addr: a,
                name,
            } => {
                let mut args: Vec<String> = pre.iter().map(|s| s.to_string()).collect();
                args.push(addr(a));
                Plan {
                    program: client.clone(),
                    args,
                    raw: None,
                    what: format!("{name} to {ip}"),
                }
            }
        }
    }
}

#[cfg(windows)]
fn find() -> Clients {
    let root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let sys = root.join("System32");
    let cmd = sys.join("cmd.exe");
    let ssh = sys.join("OpenSSH").join("ssh.exe");
    // cmd.exe reads its own command line: a path it could misread isn't used.
    let plain = |p: &std::path::Path| {
        !p.to_string_lossy()
            .chars()
            .any(|c| matches!(c, '"' | '%' | '^' | '&' | '|' | '<' | '>' | '!'))
    };
    Clients {
        ssh: (cmd.is_file() && ssh.is_file() && plain(&ssh)).then_some(Client::WinSsh { cmd, ssh }),
        rdp: Some(sys.join("mstsc.exe"))
            .filter(|p| p.is_file())
            .map(Client::Mstsc),
    }
}

#[cfg(not(windows))]
fn find() -> Clients {
    use crate::net::system_tool;
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let term = terminal(&desktop, system_tool);
    let ssh = system_tool("ssh")
        .zip(term.clone())
        .map(|(client, term)| Client::InTerminal {
            term,
            client,
            pre: &[],
            addr: "{}",
        });
    // Remmina and SDL FreeRDP ask for the log-in in a window of their own;
    // xfreerdp asks on the command line, so it runs in a terminal.
    let direct = [
        ("remmina", &["-c"][..], "rdp://{}", "Remmina"),
        ("sdl-freerdp3", &[][..], "/v:{}", "FreeRDP"),
        ("sdl-freerdp", &[][..], "/v:{}", "FreeRDP"),
    ]
    .into_iter()
    .find_map(|(bin, pre, addr, name)| {
        system_tool(bin).map(|client| Client::Direct {
            client,
            pre,
            addr,
            name,
        })
    });
    let rdp = direct.or_else(|| {
        let term = term?;
        ["xfreerdp3", "xfreerdp"]
            .into_iter()
            .find_map(system_tool)
            .map(|client| Client::InTerminal {
                term,
                client,
                pre: &[],
                addr: "/v:{}",
            })
    });
    Clients { ssh, rdp }
}

/// The terminal emulator: the desktop's own first, then the common ones.
#[cfg(not(windows))]
fn terminal(desktop: &str, find: impl Fn(&str) -> Option<PathBuf>) -> Option<Terminal> {
    // (program, name, arguments before the command)
    const KNOWN: &[(&str, &str, &[&str])] = &[
        ("konsole", "Konsole", &["-e"]),
        ("ptyxis", "Ptyxis", &["--new-window", "--"]),
        ("gnome-terminal", "GNOME Terminal", &["--"]),
        ("xfce4-terminal", "the Xfce terminal", &["-x"]),
        ("x-terminal-emulator", "the terminal", &["-e"]),
        ("kitty", "kitty", &[]),
        ("alacritty", "Alacritty", &["-e"]),
        ("foot", "foot", &[]),
        ("wezterm", "WezTerm", &["start", "--"]),
        ("xterm", "xterm", &["-e"]),
    ];
    let d = desktop.to_ascii_uppercase();
    let first: &[&str] = if d.contains("KDE") {
        &["konsole"]
    } else if d.contains("GNOME") {
        &["ptyxis", "gnome-terminal"]
    } else if d.contains("XFCE") {
        &["xfce4-terminal"]
    } else {
        &[]
    };
    let order = first
        .iter()
        .copied()
        .chain(KNOWN.iter().map(|k| k.0).filter(|p| !first.contains(p)));
    for prog in order {
        if let Some(path) = find(prog)
            && let Some((_, name, run)) = KNOWN.iter().find(|k| k.0 == prog)
        {
            return Some(Terminal { path, name, run });
        }
    }
    None
}

#[cfg(windows)]
fn run(plan: &Plan) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new(&plan.program);
    match &plan.raw {
        Some(raw) => {
            cmd.raw_arg(raw);
            // A console of its own: this app has none to share.
            cmd.creation_flags(0x0000_0010);
        }
        None => {
            cmd.args(&plan.args);
        }
    }
    cmd.spawn().map(drop).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn run(plan: &Plan) -> Result<(), String> {
    let mut cmd = std::process::Command::new(&plan.program);
    cmd.args(&plan.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::widgets::host_libraries(&mut cmd);
    // Left to run: the terminal or client is the user's now.
    cmd.spawn().map(drop).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP: Ipv4Addr = Ipv4Addr::new(10, 20, 0, 1);

    /// The only argument not written into this file is the address.
    fn only_the_ip_is_from_the_map(p: &Plan, fixed: &[&str]) {
        for a in &p.args {
            let a = a.as_str();
            assert!(
                fixed.contains(&a)
                    || a == "10.20.0.1"
                    || a == "/v:10.20.0.1"
                    || a == "rdp://10.20.0.1"
                    || a.starts_with('/') && !a.contains(' ') && !a.contains("10.20"),
                "unexpected argument {a:?}"
            );
        }
    }

    fn term(run: &'static [&'static str]) -> Terminal {
        Terminal {
            path: "/usr/bin/konsole".into(),
            name: "Konsole",
            run,
        }
    }

    #[test]
    fn ssh_in_a_terminal_passes_the_address_as_an_argument() {
        let c = Client::InTerminal {
            term: term(&["-e"]),
            client: "/usr/bin/ssh".into(),
            pre: &[],
            addr: "{}",
        };
        let p = c.plan(IP);
        assert_eq!(p.program, PathBuf::from("/usr/bin/konsole"));
        assert_eq!(
            p.args,
            ["-e", "/bin/sh", "-c", HOLD, "/usr/bin/ssh", "10.20.0.1"]
        );
        // The script is fixed text: the address isn't in it.
        assert!(!HOLD.contains("10.20"));
        only_the_ip_is_from_the_map(&p, &["-e", "-c", HOLD]);
        assert_eq!(p.what, "ssh 10.20.0.1 in Konsole");
    }

    #[test]
    fn rdp_clients() {
        let p = Client::InTerminal {
            term: term(&["--new-window", "--"]),
            client: "/usr/bin/xfreerdp".into(),
            pre: &[],
            addr: "/v:{}",
        }
        .plan(IP);
        assert_eq!(p.args.last().unwrap(), "/v:10.20.0.1");
        only_the_ip_is_from_the_map(&p, &["--new-window", "--", "-c", HOLD]);

        let p = Client::Direct {
            client: "/usr/bin/remmina".into(),
            pre: &["-c"],
            addr: "rdp://{}",
            name: "Remmina",
        }
        .plan(IP);
        assert_eq!(p.args, ["-c", "rdp://10.20.0.1"]);
        assert_eq!(p.what, "Remmina to 10.20.0.1");

        let p = Client::Mstsc(r"C:\Windows\System32\mstsc.exe".into()).plan(IP);
        assert_eq!(p.args, ["/v:10.20.0.1"]);
        assert_eq!(p.raw, None);
    }

    #[test]
    fn windows_ssh_command_line() {
        let p = Client::WinSsh {
            cmd: r"C:\Windows\System32\cmd.exe".into(),
            ssh: r"C:\Windows\System32\OpenSSH\ssh.exe".into(),
        }
        .plan(IP);
        assert!(p.args.is_empty());
        assert_eq!(
            p.raw.as_deref(),
            Some(r#"/s /k ""C:\Windows\System32\OpenSSH\ssh.exe" 10.20.0.1""#)
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn the_desktops_own_terminal_comes_first() {
        let have = |names: &'static [&'static str]| {
            move |n: &str| {
                names
                    .contains(&n)
                    .then(|| PathBuf::from(format!("/usr/bin/{n}")))
            }
        };
        let all = &["xterm", "konsole", "gnome-terminal", "ptyxis"][..];
        assert_eq!(terminal("KDE", have(all)).unwrap().name, "Konsole");
        assert_eq!(terminal("GNOME", have(all)).unwrap().name, "Ptyxis");
        assert_eq!(
            terminal("ubuntu:GNOME", have(&["gnome-terminal", "xterm"]))
                .unwrap()
                .name,
            "GNOME Terminal"
        );
        assert_eq!(terminal("", have(&["xterm"])).unwrap().name, "xterm");
        assert_eq!(terminal("KDE", have(&[])), None);
    }

    #[test]
    fn ports() {
        assert_eq!(Kind::Ssh.port(), 22);
        assert_eq!(Kind::Rdp.port(), 3389);
    }
}
