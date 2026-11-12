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
//! go on is its vendor and open ports; and a card's uplink, when it
//! answered on the network but no backup or config says which port it
//! hangs off.

/// The word, everywhere.
pub const WORD: &str = "inferred";

/// Where an inferred role came from.
pub const ROLE: &str = "Role inferred from its vendor and open ports: nothing read names it";

/// Where an inferred uplink came from, when its subnet placed it.
pub const UPLINK_SUBNET: &str = "Uplink inferred from its subnet: other cards there hang off it";

/// Where an inferred uplink came from, otherwise.
pub const UPLINK: &str = "Uplink inferred: it answered here, but no backup or config says which \
                          port it's on";

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
