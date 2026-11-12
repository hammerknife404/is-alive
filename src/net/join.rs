//! Whether this computer is joined to a domain, and to which: Active
//! Directory, Microsoft Entra ID (Azure AD), both (hybrid), only registered
//! with a work account, or none. Windows asks `dsregcmd /status`, as a tech
//! would by hand; Linux asks realmd (`realm list`). Local only: nothing is
//! sent anywhere, though dsregcmd's own diagnostics may contact the domain.

use std::process::Command;

/// Results (CSV export, the saved log): one row per field shown.
pub const COLUMNS: &[&str] = &["section", "field", "value"];

/// How the computer is joined.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
// Hybrid, Entra and Registered come from dsregcmd: Windows only.
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Kind {
    /// Active Directory and Entra ID (hybrid joined).
    Hybrid,
    /// Active Directory (or, on Linux, a Kerberos realm through realmd).
    Domain,
    /// Entra ID only (cloud joined).
    Entra,
    /// Not joined, but a work account is registered (Entra registered).
    Registered,
    /// Not joined to anything: a workgroup computer.
    Workgroup,
    /// Couldn't tell: the tool is missing or didn't answer.
    #[default]
    Unknown,
}

impl Kind {
    /// "Hybrid joined (AD + Entra ID)", for the tab's headline.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Hybrid => "Hybrid joined: Active Directory and Entra ID",
            Kind::Domain => "Joined to Active Directory",
            Kind::Entra => "Joined to Entra ID",
            Kind::Registered => "Not joined; a work account is registered (Entra registered)",
            Kind::Workgroup => "Not joined to a domain (workgroup)",
            Kind::Unknown => "Couldn't tell",
        }
    }

    /// One word for the log's table.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Hybrid => "hybrid",
            Kind::Domain => "domain",
            Kind::Entra => "entra",
            Kind::Registered => "registered",
            Kind::Workgroup => "workgroup",
            Kind::Unknown => "unknown",
        }
    }

    pub fn joined(self) -> bool {
        matches!(self, Kind::Hybrid | Kind::Domain | Kind::Entra)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Join {
    pub kind: Kind,
    /// The AD domain's DNS name ("corp.example.com") where known.
    pub dns: Option<String>,
    /// The AD domain's short (NetBIOS) name ("CORP"), from dsregcmd.
    pub netbios: Option<String>,
    /// The Entra tenant's name ("Example Ltd").
    pub tenant: Option<String>,
    /// Every field the tool printed, as (section, field, value), in order.
    pub fields: Vec<(String, String, String)>,
    /// What was asked: "dsregcmd /status".
    pub source: &'static str,
    /// The tool's output as printed, for Copy.
    pub raw: String,
    /// Why it couldn't tell.
    pub error: Option<String>,
}

/// The fields that go to Results (and from there to the Site report and
/// the Site Bundle): what says how the machine is joined and whether that
/// join works. dsregcmd prints more, the signed-in user's account among
/// it: the tab shows all of it, Results only these.
const KEEP: &[&str] = &[
    // dsregcmd
    "AzureAdJoined",
    "EnterpriseJoined",
    "DomainJoined",
    "DomainName",
    "Device Name",
    "DeviceId",
    "DeviceAuthStatus",
    "TpmProtected",
    "TenantName",
    "TenantId",
    "MdmUrl",
    "WorkplaceJoined",
    "AzureAdPrt",
    "EnterprisePrt",
    "OnPremTgt",
    "CloudTgt",
    "DRS Discovery Test",
    "AD Connectivity Test",
    "DC Test",
    // realm list
    "realm-name",
    "domain-name",
    "configured",
    "server-software",
    "client-software",
    "login-policy",
    // ours
    "DNS domain",
];

/// Whether Results keeps a field of this name.
pub fn keeps(field: &str) -> bool {
    KEEP.contains(&field)
}

impl Join {
    /// The domain to name: the AD domain's DNS name, else its short name,
    /// else the Entra tenant.
    pub fn domain(&self) -> Option<&str> {
        self.dns
            .as_deref()
            .or(self.netbios.as_deref())
            .or(self.tenant.as_deref())
    }

    /// A few words for the status bar: "corp.example.com", "Entra · Example
    /// Ltd", "workgroup".
    pub fn short(&self) -> String {
        let ad = self.dns.as_deref().or(self.netbios.as_deref());
        match self.kind {
            Kind::Hybrid => format!("{} + Entra", ad.unwrap_or("AD")),
            Kind::Domain => ad.unwrap_or("joined").to_string(),
            Kind::Entra => match &self.tenant {
                Some(t) => format!("Entra · {t}"),
                None => "Entra ID".into(),
            },
            Kind::Registered => "none (registered)".into(),
            Kind::Workgroup => "none".into(),
            Kind::Unknown => "unknown".into(),
        }
    }

    /// The fields Results keeps, in the order printed.
    fn kept(&self) -> impl Iterator<Item = &(String, String, String)> {
        self.fields.iter().filter(|(_, k, _)| keeps(k))
    }

    /// Text for the Results log: the verdict, then the fields kept.
    pub fn summary(&self) -> String {
        let mut s = format!("{}\n", self.kind.name());
        if let Some(d) = self.domain() {
            s += &format!("Domain   {d}\n");
        }
        if let Some(e) = &self.error {
            s += &format!("Note     {e}\n");
        }
        s += &format!("Asked    {}\n", self.source);
        let mut section = "";
        for (sec, k, v) in self.kept() {
            if sec != section {
                s += &format!("\n{sec}\n");
                section = sec;
            }
            s += &format!("  {k:<22}{v}\n");
        }
        s
    }

    pub fn table(&self) -> crate::log::Table {
        let row = |a: &str, b: &str, c: &str| vec![a.to_string(), b.to_string(), c.to_string()];
        let mut rows = vec![
            row("join", "kind", self.kind.tag()),
            row("join", "domain", self.domain().unwrap_or("")),
        ];
        rows.extend(self.kept().map(|(s, k, v)| row(s, k, v)));
        crate::log::Table {
            columns: COLUMNS,
            rows,
        }
    }
}

/// Ask the system. Takes a moment (dsregcmd tests the domain's reach), so
/// call it off the UI thread.
pub fn status() -> Join {
    let mut j = ask();
    if let Some(d) = dns_domain() {
        // A workgroup computer can carry a DNS suffix too: only a joined
        // one's names its domain.
        if matches!(j.kind, Kind::Hybrid | Kind::Domain) && j.dns.is_none() {
            j.dns = Some(d.clone());
        }
        j.fields
            .push(("This computer".into(), "DNS domain".into(), d));
    }
    j
}

#[cfg(windows)]
fn ask() -> Join {
    let root = std::env::var_os("WINDIR").map_or("C:\\Windows".into(), std::path::PathBuf::from);
    let mut cmd = Command::new(root.join("System32").join("dsregcmd.exe"));
    cmd.arg("/status");
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        cmd.creation_flags(0x0800_0000);
    }
    let source = "dsregcmd /status";
    match cmd.output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut j = parse_dsregcmd(&text);
            if j.kind == Kind::Unknown {
                let err = String::from_utf8_lossy(&out.stderr);
                j.error = Some(if err.trim().is_empty() {
                    format!(
                        "dsregcmd printed nothing it could read (exit {})",
                        out.status
                    )
                } else {
                    super::sanitize(err.trim())
                });
            }
            Join { source, ..j }
        }
        Err(e) => Join {
            source,
            error: Some(format!("couldn't run dsregcmd: {e}")),
            ..Join::default()
        },
    }
}

#[cfg(not(windows))]
fn ask() -> Join {
    let mut cmd = Command::new("realm");
    cmd.args(["list"]);
    crate::widgets::host_libraries(&mut cmd);
    let source = "realm list";
    match cmd.output() {
        Ok(out) if out.status.success() => Join {
            source,
            ..parse_realm(&String::from_utf8_lossy(&out.stdout))
        },
        Ok(out) => Join {
            source,
            error: Some(format!(
                "realm list failed: {}",
                super::sanitize(String::from_utf8_lossy(&out.stderr).trim())
            )),
            ..Join::default()
        },
        Err(_) => Join {
            source,
            error: Some(
                "realmd isn't installed (no `realm` command), so this can't tell. A computer \
                 joined with winbind or by hand isn't seen either."
                    .into(),
            ),
            ..Join::default()
        },
    }
}

/// The computer's primary DNS suffix: its AD domain when joined.
#[cfg(windows)]
fn dns_domain() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{
        ComputerNameDnsDomain, GetComputerNameExW,
    };
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    // SAFETY: buf holds len UTF-16 units; the call writes at most that many
    // and sets len to the count written, without the terminator.
    let ok = unsafe { GetComputerNameExW(ComputerNameDnsDomain, buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]);
    let s = super::sanitize(s.trim());
    (!s.is_empty()).then_some(s)
}

#[cfg(not(windows))]
fn dns_domain() -> Option<String> {
    None
}

#[cfg_attr(not(windows), allow(dead_code))]
/// Read `dsregcmd /status`: sections under `| Name |` banners, fields as
/// `Key : Value`.
pub fn parse_dsregcmd(text: &str) -> Join {
    let mut j = Join {
        source: "dsregcmd /status",
        raw: text.to_string(),
        ..Join::default()
    };
    let mut section = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('+') {
            continue;
        }
        if let Some(name) = t.strip_prefix('|').and_then(|s| s.strip_suffix('|')) {
            section = super::sanitize(name.trim());
            continue;
        }
        let Some((k, v)) = t.split_once(" : ") else {
            continue;
        };
        let (k, v) = (super::sanitize(k.trim()), super::sanitize(v.trim()));
        if k.is_empty() {
            continue;
        }
        j.fields.push((section.clone(), k, v));
    }
    let get = |key: &str| {
        j.fields
            .iter()
            .find(|(_, k, _)| k == key)
            .map(|(_, _, v)| v.clone())
            .filter(|v| !v.is_empty() && v != "-")
    };
    let yes = |key: &str| get(key).is_some_and(|v| v.eq_ignore_ascii_case("YES"));
    let known = ["AzureAdJoined", "DomainJoined", "WorkplaceJoined"]
        .iter()
        .any(|k| get(k).is_some());
    let (ad, entra) = (
        yes("DomainJoined") || yes("EnterpriseJoined"),
        yes("AzureAdJoined"),
    );
    j.kind = match (known, ad, entra) {
        (false, ..) => Kind::Unknown,
        (_, true, true) => Kind::Hybrid,
        (_, true, false) => Kind::Domain,
        (_, false, true) => Kind::Entra,
        _ if yes("WorkplaceJoined") => Kind::Registered,
        _ => Kind::Workgroup,
    };
    if ad {
        j.netbios = get("DomainName");
        // "PC01.corp.example.com": the domain is what follows the name.
        j.dns = get("Device Name")
            .and_then(|n| n.split_once('.').map(|(_, d)| d.to_ascii_lowercase()))
            .filter(|d| !d.is_empty());
    }
    if entra || yes("WorkplaceJoined") {
        j.tenant = get("TenantName");
    }
    j
}

#[cfg_attr(windows, allow(dead_code))]
/// Read `realm list`: a block per realm, its name unindented, then
/// `key: value` lines. The first realm this computer is a member of counts;
/// none listed means not joined.
pub fn parse_realm(text: &str) -> Join {
    let mut j = Join {
        source: "realm list",
        raw: text.to_string(),
        kind: Kind::Workgroup,
        ..Join::default()
    };
    let mut realm = String::new();
    let mut member: Option<String> = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            realm = super::sanitize(line.trim());
            continue;
        }
        let Some((k, v)) = line.trim().split_once(':') else {
            continue;
        };
        let (k, v) = (super::sanitize(k.trim()), super::sanitize(v.trim()));
        if k == "configured" && v.contains("member") && member.is_none() {
            member = Some(realm.clone());
        }
        j.fields.push((realm.clone(), k, v));
    }
    if let Some(m) = member {
        let get = |key: &str| {
            j.fields
                .iter()
                .find(|(s, k, _)| *s == m && k == key)
                .map(|(_, _, v)| v.clone())
        };
        j.kind = Kind::Domain;
        j.dns = get("domain-name").or(Some(m.to_ascii_lowercase()));
        j.netbios = get("realm-name").filter(|r| !r.eq_ignore_ascii_case(&m));
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hybrid-joined PC as `dsregcmd /status` prints it (fictional
    /// names, documentation IDs), cut to the parts that matter here.
    const HYBRID: &str = "
+----------------------------------------------------------------------+
| Device State                                                         |
+----------------------------------------------------------------------+

             AzureAdJoined : YES
          EnterpriseJoined : NO
              DomainJoined : YES
                DomainName : CORP
               Device Name : PC01.corp.example.com

+----------------------------------------------------------------------+
| Device Details                                                       |
+----------------------------------------------------------------------+

                  DeviceId : 00000000-0000-0000-0000-000000000001
              TpmProtected : YES
          DeviceAuthStatus : SUCCESS

+----------------------------------------------------------------------+
| Tenant Details                                                       |
+----------------------------------------------------------------------+

                TenantName : Example Ltd
                  TenantId : 00000000-0000-0000-0000-000000000002
                    MdmUrl : https://enrollment.manage.microsoft.com/enrollmentserver/discovery.svc

+----------------------------------------------------------------------+
| User State                                                           |
+----------------------------------------------------------------------+

                    NgcSet : NO
           WorkplaceJoined : NO

+----------------------------------------------------------------------+
| SSO State                                                            |
+----------------------------------------------------------------------+

                AzureAdPrt : YES

+----------------------------------------------------------------------+
| Diagnostic Data                                                      |
+----------------------------------------------------------------------+

     Executing Account Name : CORP\\someone, someone@example.com
";

    #[test]
    fn hostile_output_never_panics() {
        crate::hostile::each_text(HYBRID, |t| {
            let _ = parse_dsregcmd(t).short();
            let _ = parse_realm(t).short();
        });
    }

    #[test]
    fn dsregcmd_hybrid_names_the_domain_and_tenant() {
        let j = parse_dsregcmd(HYBRID);
        assert_eq!(j.kind, Kind::Hybrid);
        assert_eq!(j.netbios.as_deref(), Some("CORP"));
        assert_eq!(j.dns.as_deref(), Some("corp.example.com"));
        assert_eq!(j.tenant.as_deref(), Some("Example Ltd"));
        assert_eq!(j.domain(), Some("corp.example.com"));
        assert_eq!(j.short(), "corp.example.com + Entra");
        let dev = j.fields.iter().find(|f| f.1 == "DeviceId").unwrap();
        assert_eq!(dev.0, "Device Details");
    }

    /// Results keeps how it's joined, not who is signed in.
    #[test]
    fn results_leave_out_the_signed_in_account() {
        let j = parse_dsregcmd(HYBRID);
        let text = j.summary();
        assert!(text.contains("DomainJoined"), "{text}");
        assert!(text.contains("TenantName"), "{text}");
        assert!(!text.contains("someone"), "{text}");
        let t = j.table();
        assert_eq!(t.columns, COLUMNS);
        assert_eq!(t.rows[0], ["join", "kind", "hybrid"]);
        assert_eq!(t.rows[1], ["join", "domain", "corp.example.com"]);
        assert!(t.rows.iter().all(|r| !r.join(" ").contains("someone")));
        assert!(j.raw.contains("someone"), "the tab still shows it all");
    }

    #[test]
    fn dsregcmd_each_kind_of_join() {
        let flags = |aad: &str, ad: &str, wp: &str| {
            format!(
                "| Device State |\n AzureAdJoined : {aad}\n DomainJoined : {ad}\n \
                 DomainName : CORP\n| Tenant Details |\n TenantName : Example Ltd\n\
                 | User State |\n WorkplaceJoined : {wp}\n"
            )
        };
        let kind = |a, b, c| parse_dsregcmd(&flags(a, b, c)).kind;
        assert_eq!(kind("NO", "YES", "NO"), Kind::Domain);
        assert_eq!(kind("YES", "NO", "NO"), Kind::Entra);
        assert_eq!(kind("NO", "NO", "YES"), Kind::Registered);
        assert_eq!(kind("NO", "NO", "NO"), Kind::Workgroup);
        let entra = parse_dsregcmd(&flags("YES", "NO", "NO"));
        assert_eq!(entra.short(), "Entra · Example Ltd");
        assert_eq!(entra.netbios, None, "DomainName only counts when AD joined");
        // No Device Name: the short name stands in.
        assert_eq!(parse_dsregcmd(&flags("NO", "YES", "NO")).short(), "CORP");
        assert_eq!(parse_dsregcmd(&flags("NO", "NO", "NO")).short(), "none");
    }

    #[test]
    fn dsregcmd_that_says_nothing_is_unknown() {
        let j = parse_dsregcmd("The system cannot find the file specified.\n");
        assert_eq!(j.kind, Kind::Unknown);
        assert_eq!(j.short(), "unknown");
        assert!(!j.kind.joined());
    }

    #[test]
    fn realm_list_member_and_not() {
        let text = "corp.example.com\n  type: kerberos\n  realm-name: CORP.EXAMPLE.COM\n  \
                    domain-name: corp.example.com\n  configured: kerberos-member\n  \
                    server-software: active-directory\n  client-software: sssd\n  \
                    login-formats: %U@corp.example.com\n";
        let j = parse_realm(text);
        assert_eq!(j.kind, Kind::Domain);
        assert_eq!(j.dns.as_deref(), Some("corp.example.com"));
        assert!(j.summary().contains("client-software"));
        assert!(!j.summary().contains("login-formats"));
        // Discovered but not joined.
        let seen = parse_realm(
            "corp.example.com\n  type: kerberos\n  domain-name: corp.example.com\n  \
             configured: no\n",
        );
        assert_eq!(seen.kind, Kind::Workgroup);
        assert_eq!(parse_realm("").kind, Kind::Workgroup);
    }
}
