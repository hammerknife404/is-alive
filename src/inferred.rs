//! One mark for what is-alive worked out and didn't read or measure.
//!
//! A measured value is plain. An inferred one carries the word
//! "inferred", drawn the same small way wherever it shows (a card's tag,
//! the inspector, the legend, the site report, a map's CSV, the bundle's
//! card notes), and says where it came from: on hover in the app and the
//! report, in a field of its own in a file. It used to be "guessed" in
//! one place, "assumed" in another and "inferred" in a third.
//!
//! Two things are inferred today: a device's role, when all there is to
//! go on is its vendor, open ports or web page (or a /mapnetwork map
//! says it worked the role out); and a card's uplink, when no backup or
//! config says which port it hangs off. Each says why in its own words:
//! the subnet placed it, the map drew it dashed, the pair was reciprocal,
//! it was last seen there, or nothing links to it.

/// The word, everywhere.
pub const WORD: &str = "inferred";

/// Where an inferred role came from, when nothing narrower is known.
pub const ROLE: &str = "Role inferred from its vendor and open ports: nothing read names it";

/// Where an inferred role came from: what the sweep had to go on.
pub fn role_from(vendor: bool, ports: bool, page: bool) -> String {
    let mut from = Vec::new();
    if vendor {
        from.push("its vendor");
    }
    if ports {
        from.push("its open ports");
    }
    if page {
        from.push("its web page");
    }
    let from = match from.as_slice() {
        [] => return ROLE.to_string(),
        [a] => a.to_string(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("Role inferred from {from}: nothing read names it")
}

/// A /mapnetwork map's card whose role the map itself marks inferred.
pub const ROLE_MAP: &str =
    "Role inferred on the /mapnetwork map: nothing the map was made from names it";

/// Where an inferred uplink came from, when its subnet placed it.
pub const UPLINK_SUBNET: &str = "Uplink inferred from its subnet: other cards there hang off it";

/// Where an inferred uplink came from, otherwise.
pub const UPLINK: &str = "Uplink inferred: it answered here, but no backup or config says which \
                          port it's on";

/// A /mapnetwork map drew the link dashed: it worked it out.
pub const UPLINK_MAP: &str = "Uplink inferred on the /mapnetwork map (drawn dashed): the map doesn't confirm this link, even where it names a port";

/// A /mapnetwork map's reciprocal pair: the direction is the map's pick.
pub const UPLINK_RECIPROCAL: &str = "Uplink direction inferred: the controller's uplink data \
                                     points both ways, so the map picked one";

/// Where the controller last saw it, not where it is now.
pub const UPLINK_LAST_SEEN: &str =
    "Uplink inferred from where it was last seen: not seen live when the map was made";

/// A card the map links to nothing.
pub const UPLINK_NONE: &str =
    "Placement unknown: the /mapnetwork map links nothing to it, so where it plugs in isn't known";

/// Why a card's uplink is inferred, from its own detail line when it has
/// one ("Uplink inferred from its subnet…", "Placement unknown…"), else
/// the general one.
pub fn uplink_why(details: &[String]) -> &str {
    details
        .iter()
        .find(|d| d.starts_with("Uplink ") || d.starts_with("Placement unknown"))
        .map_or(UPLINK, String::as_str)
}

/// The mark in the site report: the word, with where it came from on
/// hover. `why` is escaped here.
pub fn html(why: &str) -> String {
    format!(
        "<i class=\"inferred\" title=\"{}\">{WORD}</i>",
        crate::markdown_out::escape(why)
    )
}

/// The mark in the app, as a card's tags are drawn: small and muted,
/// with where it came from on hover.
pub fn mark(ui: &mut eframe::egui::Ui, why: &str) -> eframe::egui::Response {
    ui.label(eframe::egui::RichText::new(WORD).small().weak())
        .on_hover_text(why)
}

#[cfg(test)]
mod tests {
    /// One word for it. A file that words an inferred value another way
    /// ("guessed", "assumed") in something the app shows or writes fails
    /// here. `Link::Assumed`, and "assumed" as a saved map's own key for
    /// it, stay: files saved by earlier versions say so.
    #[test]
    fn nothing_the_app_says_calls_it_guessed_or_assumed() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        let words = [["gues", "sed"].concat(), ["assu", "med"].concat()];
        let mut found = Vec::new();
        for f in files {
            let name = f.strip_prefix(&src).unwrap().display().to_string();
            for (n, line) in std::fs::read_to_string(&f).unwrap().lines().enumerate() {
                let t = line.trim();
                // Comments, test names and asserts may say what they like.
                if t.starts_with("//") || t.starts_with("fn ") || t.contains("assert") {
                    continue;
                }
                // Only what's written in a string is said to anyone.
                let said: String = t
                    .split('"')
                    .skip(1)
                    .step_by(2)
                    .collect::<Vec<_>>()
                    .join(" ");
                let said = said.to_lowercase();
                // The saved-map key, read and written as it always was.
                if name.ends_with("saved.rs") && said.trim() == words[1] {
                    continue;
                }
                if words.iter().any(|w| said.contains(w.as_str())) {
                    found.push(format!("{name}:{}: {t}", n + 1));
                }
            }
        }
        assert!(
            found.is_empty(),
            "say inferred (src/inferred.rs):\n{}",
            found.join("\n")
        );
    }
}
