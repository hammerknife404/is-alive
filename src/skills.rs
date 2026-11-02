//! The /unifidump and /mapnetwork skills, embedded (from `skills/`, which
//! `scripts/sync-skills.py` copies from the source skills and scrubs).
//!
//! A Site Bundle carries them so a Claude chat without the skills installed
//! can still follow them, and their two worked example maps open in the Map
//! tab as examples, with a blank one built from the skill's template.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/skills.rs"));

/// The skills a bundle carries, by folder name.
pub const NAMES: [&str; 2] = ["unifidump", "mapnetwork"];

/// The example maps: (menu label, file under skills/, or `BLANK`).
pub const EXAMPLES: [(&str, &str); 3] = [
    (
        "Acme Distribution: one campus, HA firewalls, dual WAN",
        "mapnetwork/assets/example-Acme_Distribution_LLC.dc.html",
    ),
    (
        "Contoso Health: 23 clinics, 81 devices",
        "mapnetwork/assets/example-Contoso_Health.dc.html",
    ),
    (BLANK_LABEL, BLANK),
];

/// The blank example: not a file under skills/ but built from the
/// /mapnetwork template, its sample site swapped for the smallest map that
/// still draws (the Internet and one gateway at one site). A start for a
/// map drawn by hand, or a small map to try things on.
pub const BLANK: &str = "Blank_map.dc.html";
pub const BLANK_LABEL: &str = "Blank map: the Internet and one gateway";
/// The template the blank is built in.
const TEMPLATE: &str = "mapnetwork/assets/Site Map Template.dc.html";

/// The blank map's site data, in the template's own format.
const BLANK_DATA: &str = r#"    // BLANK MAP: the Internet and one gateway at one site. Add to it, or
    // replace this whole block, to map a network.
    this.META = {
      title: 'Blank map',
      subtitle: 'the Internet and one gateway · a template to start from'
    };

    this.SITES = {
      site: { name:'Site' }
    };

    this.ZONES = {
      core: { name:'Edge / routing', vlan:'WAN · firewall', dark:'#5b9dff', light:'#2563c9' }
    };

    this.NODES = [
      { id:'internet', zone:'core', kind:'WAN', label:'Internet', sub:'upstream',
        details:[], flags:[], chips:[] },
      { id:'gateway', site:'site', zone:'core', kind:'GATEWAY · FIREWALL', label:'Gateway', sub:'',
        details:[], flags:[], chips:[] }
    ];

    this.EDGES = [
      { id:'e1', from:'internet', to:'gateway', cls:'backbone' }
    ];

    this.ACTIONS = [];

    this.BANDS = [];
"#;

/// The blank map's page: the template with its site data block swapped
/// for `BLANK_DATA` (so it still opens in a browser, and /mapnetwork can
/// fill it in), or just the data if the template has lost its markers.
fn blank() -> &'static [u8] {
    static PAGE: OnceLock<Vec<u8>> = OnceLock::new();
    PAGE.get_or_init(|| {
        let html = file(TEMPLATE)
            .map(|d| String::from_utf8_lossy(d).into_owned())
            .unwrap_or_default();
        let start = html
            .find("SITE DATA ====")
            .and_then(|s| Some(s + html[s..].find('\n')? + 1));
        let end = start.and_then(|s| {
            let e = s + html[s..].find("END SITE DATA")?;
            Some(html[..e].rfind('\n')? + 1)
        });
        match (start, end) {
            (Some(s), Some(e)) if s <= e => format!("{}{BLANK_DATA}{}", &html[..s], &html[e..]),
            _ => format!(
                "<html><script>\n    /* ===================== SITE DATA ===================== */\n\
                 {BLANK_DATA}    /* =================== END SITE DATA =================== */\n\
                 </script></html>\n"
            ),
        }
        .into_bytes()
    })
}

/// An example's bytes, by its `EXAMPLES` file.
pub fn example_file(f: &str) -> Option<&'static [u8]> {
    if f == BLANK { Some(blank()) } else { file(f) }
}

/// True when `p` is the blank example opened (a map saved from it is a
/// plan).
pub fn is_blank(p: &Path) -> bool {
    example_path(EXAMPLES.len() - 1) == p
}

/// Where an example "is" for Config Export: not a real file, so a Reload
/// reads it from here again.
const EXAMPLE_DIR: &str = "(is-alive example)";

pub fn file(path: &str) -> Option<&'static [u8]> {
    FILES.iter().find(|(p, _)| *p == path).map(|(_, d)| *d)
}

/// The path Config Export opens example `i` under.
pub fn example_path(i: usize) -> PathBuf {
    let f = EXAMPLES[i].1;
    Path::new(EXAMPLE_DIR).join(f.rsplit('/').next().unwrap_or(f))
}

/// An example's bytes, if `p` is one of `example_path`'s.
pub fn example(p: &Path) -> Option<&'static [u8]> {
    if p.parent() != Some(Path::new(EXAMPLE_DIR)) {
        return None;
    }
    let name = p.file_name()?.to_str()?;
    EXAMPLES
        .iter()
        .find(|(_, f)| f.rsplit('/').next() == Some(name))
        .and_then(|(_, f)| example_file(f))
}

/// Total size of the skills' files.
pub fn bytes() -> usize {
    FILES.iter().map(|(_, d)| d.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_skills_and_both_examples_are_embedded() {
        for n in NAMES {
            assert!(file(&format!("{n}/SKILL.md")).is_some(), "{n}");
        }
        for i in 0..EXAMPLES.len() {
            let d = example(&example_path(i)).expect("example");
            assert!(crate::mapnetwork::sniff(d));
            assert!(crate::mapnetwork::parse(d).is_ok());
        }
        assert!(example(Path::new("/tmp/example-Contoso_Health.dc.html")).is_none());
    }

    #[test]
    fn the_blank_example_is_the_template_with_two_cards() {
        let i = EXAMPLES.iter().position(|e| e.1 == BLANK).unwrap();
        assert!(is_blank(&example_path(i)));
        assert!(!is_blank(&example_path(0)));
        let d = example(&example_path(i)).unwrap();
        let text = std::str::from_utf8(d).unwrap();
        // Built in the template (its engine kept, its sample site gone).
        let template = std::str::from_utf8(file(TEMPLATE).unwrap()).unwrap();
        assert!(
            text.len() > template.len() / 2,
            "the template lost its markers"
        );
        assert!(!text.contains("Acme Field Office"));
        let doc = crate::mapnetwork::parse(d).unwrap();
        assert_eq!(doc.title, "Blank map");
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.edges.len(), 1);
        assert_eq!(doc.sites.len(), 1);
        assert!(doc.actions.is_empty());
    }
}
