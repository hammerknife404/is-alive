//! The /unifidump and /mapnetwork skills, embedded (from `skills/`, which
//! `scripts/sync-skills.py` copies from the source skills and scrubs).
//!
//! A Site Bundle carries them so a Claude chat without the skills installed
//! can still follow them, and their two worked example maps open in the Map
//! tab as examples.

use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/skills.rs"));

/// The skills a bundle carries, by folder name.
pub const NAMES: [&str; 2] = ["unifidump", "mapnetwork"];

/// The example maps: (menu label, file under skills/).
pub const EXAMPLES: [(&str, &str); 2] = [
    (
        "Acme Distribution: one campus, HA firewalls, dual WAN",
        "mapnetwork/assets/example-Acme_Distribution_LLC.dc.html",
    ),
    (
        "Contoso Health: 23 clinics, 81 devices",
        "mapnetwork/assets/example-Contoso_Health.dc.html",
    ),
];

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
        .find(|(_, f)| f.ends_with(&format!("/{name}")))
        .and_then(|(_, f)| file(f))
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
}
