//! A /mapnetwork map (`<Client>.dc.html`) read back in, so the Map tab can
//! draw it in this app's own style. The file is the skill's engine with the
//! site's data inlined between two markers as JavaScript object literals
//! (`this.NODES = [ … ]`); only that block is read, and only as data: it's
//! parsed as literals, never run.

use serde_json::{Map, Value, json};

const START: &str = "SITE DATA ====";
const END: &str = "END SITE DATA";

pub struct Doc {
    pub title: String,
    pub subtitle: String,
    /// (key, name).
    pub sites: Vec<(String, String)>,
    /// key → (name, vlan caption).
    pub zones: Map<String, Value>,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    pub actions: Vec<Value>,
    /// The site data as JSON, for Save JSON.
    pub json: String,
    /// The page as opened, so a Site Bundle can hand it back to /mapnetwork
    /// to update rather than start over.
    pub html: String,
}

impl Doc {
    pub fn summary(&self) -> String {
        let mut s = "Format     /mapnetwork map (.dc.html)\n".to_string();
        s += &format!("Title      {}\n", self.title);
        if !self.subtitle.is_empty() {
            s += &format!("Subtitle   {}\n", self.subtitle);
        }
        s += &format!(
            "\nKept\n  {:<18}{:>6}\n  {:<18}{:>6}\n  {:<18}{:>6}\n  {:<18}{:>6}\n  {:<18}{:>6}\n",
            "sites",
            self.sites.len(),
            "zones",
            self.zones.len(),
            "devices",
            self.nodes.len(),
            "links",
            self.edges.len(),
            "findings",
            self.actions.len(),
        );
        s += "\nOnly the map's data block is read, as plain values: the page's code never runs.\n\
Open the Map tab to see it in is-alive's style; a sweep of the same network marks\n\
which devices answer.\n";
        s
    }
}

/// True when `data` looks like a /mapnetwork map.
pub fn sniff(data: &[u8]) -> bool {
    let head = &data[..data.len().min(1 << 20)];
    let text = String::from_utf8_lossy(head);
    text.contains("this.NODES") && (text.contains(START) || text.contains("<html"))
}

pub fn parse(data: &[u8]) -> Result<Doc, String> {
    let text = std::str::from_utf8(data).map_err(|_| "map file isn't UTF-8 text")?;
    let block = match text.find(START) {
        Some(s) => {
            let rest = &text[s + START.len()..];
            &rest[..rest.find(END).unwrap_or(rest.len())]
        }
        None => text,
    };
    let mut data = Map::new();
    for key in ["META", "SITES", "ZONES", "NODES", "EDGES", "ACTIONS"] {
        if let Some(v) = assignment(block, key)? {
            data.insert(key.to_lowercase(), v);
        }
    }
    let list = |k: &str| match data.get(k) {
        Some(Value::Array(a)) => a.iter().filter(|v| v.is_object()).cloned().collect(),
        _ => Vec::new(),
    };
    let nodes: Vec<Value> = list("nodes");
    if nodes.is_empty() {
        return Err("no devices (this.NODES) found: is this a /mapnetwork map?".into());
    }
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let meta = data.get("meta").cloned().unwrap_or(json!({}));
    let sites = match data.get("sites") {
        Some(Value::Object(m)) => m
            .iter()
            .map(|(k, v)| {
                let name = str_of(v, "name");
                (k.clone(), if name.is_empty() { k.clone() } else { name })
            })
            .collect(),
        _ => Vec::new(),
    };
    let zones = match data.get("zones") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    Ok(Doc {
        title: str_of(&meta, "title"),
        subtitle: str_of(&meta, "subtitle"),
        sites,
        zones,
        edges: list("edges"),
        actions: list("actions"),
        nodes,
        json: serde_json::to_string_pretty(&Value::Object(data)).unwrap_or_default(),
        html: text.to_string(),
    })
}

/// The literal assigned to `this.<key>`. A mention in a comment, a
/// comparison or the engine reading it back (`this.NODES = this.NODES…`)
/// is passed over: only `= {` or `= [` starts the data.
fn assignment(block: &str, key: &str) -> Result<Option<Value>, String> {
    let pat = format!("this.{key}");
    let mut from = 0;
    while let Some(at) = block[from..].find(&pat).map(|i| from + i) {
        from = at + pat.len();
        let rest = block[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=').filter(|r| !r.starts_with('=')) else {
            continue;
        };
        let mut p = Parser {
            s: rest.as_bytes(),
            i: 0,
            depth: 0,
        };
        p.skip();
        if !matches!(p.peek(), Some(b'{' | b'[')) {
            continue;
        }
        return p
            .value()
            .map(Some)
            .map_err(|e| format!("this.{key}: {e} (byte {})", p.i));
    }
    Ok(None)
}

/// JavaScript object/array literals: bare or quoted keys, single, double or
/// backtick strings, comments and trailing commas. Anything that isn't a
/// literal (a call, a variable) is an error, not something to evaluate.
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn skip(&mut self) {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.i += 1;
            }
            let rest = &self.s[self.i.min(self.s.len())..];
            if rest.starts_with(b"//") {
                while self.peek().is_some_and(|c| c != b'\n') {
                    self.i += 1;
                }
            } else if rest.starts_with(b"/*") {
                self.i = match rest[2..].windows(2).position(|w| w == b"*/") {
                    Some(p) => self.i + 2 + p + 2,
                    None => self.s.len(),
                };
            } else {
                return;
            }
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip();
        if self.depth > 64 {
            return Err("nested too deep".into());
        }
        match self.peek().ok_or("unexpected end")? {
            b'{' => {
                self.i += 1;
                self.depth += 1;
                let mut m = Map::new();
                loop {
                    self.skip();
                    match self.peek() {
                        Some(b'}') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => {
                            self.i += 1;
                            continue;
                        }
                        _ => {}
                    }
                    let k = match self.peek() {
                        Some(q @ (b'\'' | b'"' | b'`')) => self.string(q)?,
                        _ => self.word()?,
                    };
                    self.skip();
                    if self.peek() != Some(b':') {
                        return Err(format!("expected ':' after {k:?}"));
                    }
                    self.i += 1;
                    let v = self.value()?;
                    m.insert(k, v);
                }
                self.depth -= 1;
                Ok(Value::Object(m))
            }
            b'[' => {
                self.i += 1;
                self.depth += 1;
                let mut a = Vec::new();
                loop {
                    self.skip();
                    match self.peek() {
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => {
                            self.i += 1;
                            continue;
                        }
                        _ => a.push(self.value()?),
                    }
                }
                self.depth -= 1;
                Ok(Value::Array(a))
            }
            q @ (b'\'' | b'"' | b'`') => self.string(q).map(Value::String),
            c if c == b'-' || c == b'.' || c.is_ascii_digit() => {
                let start = self.i;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'+'))
                {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
                t.parse::<f64>()
                    .ok()
                    .and_then(|f| {
                        if f.fract() == 0.0 && f.abs() < 1e15 {
                            Some(json!(f as i64))
                        } else {
                            serde_json::Number::from_f64(f).map(Value::Number)
                        }
                    })
                    .ok_or_else(|| format!("bad number {t:?}"))
            }
            _ => match self.word()?.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                "null" | "undefined" => Ok(Value::Null),
                w => Err(format!("{w:?} isn't a plain value")),
            },
        }
    }

    fn word(&mut self) -> Result<String, String> {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'$')
        {
            self.i += 1;
        }
        if start == self.i {
            return Err(format!(
                "unexpected {:?}",
                self.peek().map(char::from).unwrap_or(' ')
            ));
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn string(&mut self, q: u8) -> Result<String, String> {
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.i += 1;
            if c == q {
                break;
            }
            if q == b'`' && c == b'$' && self.peek() == Some(b'{') {
                return Err("template string with ${…} isn't a plain value".into());
            }
            if c != b'\\' {
                out.push(c);
                continue;
            }
            let e = self.peek().ok_or("unterminated string")?;
            self.i += 1;
            let ch = match e {
                b'n' => '\n',
                b't' => '\t',
                b'r' => '\r',
                b'b' | b'f' | b'v' | b'0' => ' ',
                b'\n' => continue,
                b'u' => {
                    let hex = if self.peek() == Some(b'{') {
                        let end = self.s[self.i..]
                            .iter()
                            .position(|&c| c == b'}')
                            .ok_or("bad \\u{…}")?;
                        let h = &self.s[self.i + 1..self.i + end];
                        self.i += end + 1;
                        h
                    } else {
                        let h = self.s.get(self.i..self.i + 4).ok_or("bad \\u")?;
                        self.i += 4;
                        h
                    };
                    let mut cp = u32::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16)
                        .map_err(|_| "bad \\u")?;
                    // A surrogate pair, as JSON writes characters past U+FFFF.
                    if (0xD800..0xDC00).contains(&cp)
                        && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                        && let Some(lo) = self
                            .s
                            .get(self.i + 2..self.i + 6)
                            .and_then(|h| std::str::from_utf8(h).ok())
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .filter(|lo| (0xDC00..0xE000).contains(lo))
                    {
                        self.i += 6;
                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                    }
                    char::from_u32(cp).unwrap_or('\u{FFFD}')
                }
                b'x' => {
                    let h = self.s.get(self.i..self.i + 2).ok_or("bad \\x")?;
                    self.i += 2;
                    char::from(
                        u8::from_str_radix(std::str::from_utf8(h).unwrap_or(""), 16)
                            .map_err(|_| "bad \\x")?,
                    )
                }
                other => {
                    out.push(other);
                    continue;
                }
            };
            let mut buf = [0; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html><script>
    class X { constructor() {
    /* ===================== SITE DATA ===================== */
    // SAMPLE SITE
    this.META = { title: 'Acme — Network', subtitle: "1 site · clients not mapped" };
    this.SITES = { "hq": { "name": "Head Office" } };
    this.ZONES = {
      gateway: { name:'Gateway / firewall', vlan:'edge', dark:'#5b9dff', light:'#2563c9' },
      distribution: { name:'Switches', vlan:'wired', dark:'#a98bff', light:'#6d44d9' },
    };
    this.NODES = [
      { id:'internet', zone:'gateway', kind:'WAN', label:'Internet', sub:'upstream' },
      { id:'fw', site:'hq', zone:'gateway', kind:'FIREWALL', label:'SonicWall TZ470', sub:'10.0.0.1',
        details:['It\'s the edge', "\\\\acme\\share", 'café'], flags:[], chips:['TZ470',], vlans:[10, 20] },
      /* ---- core ---- */
      { id:'sw', site:'hq', zone:'distribution', kind:'CORE SWITCH', label:'USW-24', sub:'10.0.0.2', x: 1.5 },
    ];
    this.EDGES = [
      { id:'e1', from:'internet', to:'fw', cls:'backbone' },
      { id:'e2', from:'fw', to:'sw', cls:'wired', label:'X2', dashed:false },
    ];
    this.ACTIONS = [ { id:'a1', sev:'high', text:'No HA', nodes:['fw'] } ];
    this.BANDS = [];
    /* =================== END SITE DATA =================== */
    this.NODES.push(evil());
    } }
    </script></html>"#;

    #[test]
    fn reads_the_site_data_block() {
        assert!(sniff(PAGE.as_bytes()));
        let d = parse(PAGE.as_bytes()).unwrap();
        assert_eq!(d.title, "Acme — Network");
        assert_eq!(d.sites, vec![("hq".into(), "Head Office".into())]);
        assert_eq!(d.zones.len(), 2);
        assert_eq!(d.nodes.len(), 3);
        assert_eq!(d.edges.len(), 2);
        assert_eq!(d.actions[0]["nodes"], json!(["fw"]));
        let fw = &d.nodes[1];
        assert_eq!(
            fw["details"],
            json!(["It's the edge", "\\\\acme\\share", "café"])
        );
        assert_eq!(fw["vlans"], json!([10, 20]));
        assert_eq!(fw["chips"], json!(["TZ470"]));
        assert_eq!(d.nodes[2]["x"], json!(1.5));
        assert!(d.summary().contains("devices"));
    }

    #[test]
    fn skips_mentions_that_are_not_the_data() {
        let page = "// fill this.NODES below\nif (this.NODES == null) {}\n\
this.NODES = this.NODES || [];\nthis.NODES = [ { id:'a', label:'A' } ];";
        let d = parse(page.as_bytes()).unwrap();
        assert_eq!(d.nodes.len(), 1);
        assert_eq!(d.html, page);
    }

    #[test]
    fn refuses_code_and_junk() {
        assert!(parse(b"this.NODES = [ foo() ];").is_err());
        assert!(parse(b"this.NODES = [ { a: `x${y}` } ];").is_err());
        assert!(parse(b"<html>nothing here</html>").is_err());
        assert!(!sniff(b"<html>a page</html>"));
        let deep = format!("this.NODES = {}", "[".repeat(500));
        assert!(parse(deep.as_bytes()).is_err());
        for n in (0..PAGE.len()).step_by(37) {
            let _ = parse(&PAGE.as_bytes()[..n]);
        }
    }
}
