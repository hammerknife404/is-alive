//! Port list parsing, presets and well-known service names.

pub const PRESETS: &[(&str, &str)] = &[
    (
        "Common",
        "21-23,25,53,80,110,135,139,143,389,443,445,465,587,636,993,995,1433,1723,3306,3389,5432,5900,5985,8080,8443",
    ),
    (
        "Windows / AD",
        "53,88,135,139,389,445,464,636,3268,3269,3389,5985,5986,9389",
    ),
    ("Web", "80,443,8000,8008,8080,8443,8888"),
    // TCP only: IKE (500/4500) and OpenVPN's default 1194 are UDP, which a
    // connect scan can't test, so they'd always read filtered.
    ("Firewall / VPN", "22,443,1723,4433,8443,10443"),
    ("Mail", "25,110,143,465,587,993,995"),
    ("Well-known 1-1024", "1-1024"),
];

const SERVICES: &[(u16, &str)] = &[
    (20, "ftp-data"),
    (21, "ftp"),
    (22, "ssh"),
    (23, "telnet"),
    (25, "smtp"),
    (53, "dns"),
    (80, "http"),
    (88, "kerberos"),
    (110, "pop3"),
    (111, "rpcbind"),
    (135, "msrpc"),
    (139, "netbios-ssn"),
    (143, "imap"),
    (161, "snmp"),
    (389, "ldap"),
    (443, "https"),
    (445, "smb"),
    (464, "kpasswd"),
    (465, "smtps"),
    (500, "isakmp"),
    (514, "syslog"),
    (515, "lpd"),
    (587, "submission"),
    (631, "ipp"),
    (636, "ldaps"),
    (853, "dns-over-tls"),
    (873, "rsync"),
    (993, "imaps"),
    (995, "pop3s"),
    (1194, "openvpn"),
    (1433, "mssql"),
    (1521, "oracle"),
    (1723, "pptp"),
    (1883, "mqtt"),
    (2049, "nfs"),
    (3268, "ldap-gc"),
    (3269, "ldaps-gc"),
    (3306, "mysql"),
    (3389, "rdp"),
    (4433, "sslvpn"),
    (4500, "ipsec-nat-t"),
    (5060, "sip"),
    (5432, "postgres"),
    (5900, "vnc"),
    (5985, "winrm"),
    (5986, "winrm-https"),
    (6379, "redis"),
    (8000, "http-alt"),
    (8008, "http-alt"),
    (8080, "http-proxy"),
    (8443, "https-alt"),
    (8888, "http-alt"),
    (9100, "jetdirect"),
    (9389, "ad-web-svc"),
    (10443, "fortinet-sslvpn"),
    (25565, "minecraft"),
    (27017, "mongodb"),
];

pub fn service_name(port: u16) -> &'static str {
    SERVICES
        .binary_search_by_key(&port, |(p, _)| *p)
        .map(|i| SERVICES[i].1)
        .unwrap_or("")
}

/// Parse "22,80,443,8000-8010" into a sorted, de-duplicated port list.
pub fn parse_ports(spec: &str) -> Result<Vec<u16>, String> {
    let mut out = Vec::new();
    for part in spec
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let parse = |s: &str| {
            s.trim()
                .parse::<u16>()
                .ok()
                .filter(|&p| p != 0)
                .ok_or_else(|| format!("invalid port '{s}'"))
        };
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (parse(a)?, parse(b)?);
                if a > b {
                    return Err(format!("invalid range '{part}'"));
                }
                out.extend(a..=b);
            }
            None => out.push(parse(part)?),
        }
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err("no ports given".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lists_and_ranges() {
        assert_eq!(
            parse_ports("443, 22,80-82,22").unwrap(),
            vec![22, 80, 81, 82, 443]
        );
        assert!(parse_ports("0").is_err());
        assert!(parse_ports("90-80").is_err());
        assert!(parse_ports("abc").is_err());
        assert_eq!(parse_ports("1-65535").unwrap().len(), 65535);
    }

    #[test]
    fn services_sorted_and_found() {
        assert!(SERVICES.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(service_name(3389), "rdp");
        assert_eq!(service_name(12345), "");
    }

    #[test]
    fn presets_parse() {
        for (_, spec) in PRESETS {
            parse_ports(spec).unwrap();
        }
    }
}
