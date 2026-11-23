//! Update check: asks GitHub for the latest release and says whether it's
//! newer than this build. Nothing is downloaded or replaced; the status bar
//! links to the release page.

/// Where releases are published.
const LATEST: &str = "https://api.github.com/repos/hammerknife404/is-alive/releases/latest";
/// The releases page, for a link when the API answer has none.
pub const RELEASES: &str = "https://github.com/hammerknife404/is-alive/releases";

/// A release newer than this build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// "0.24.0", without the v.
    pub version: String,
    /// Its page on GitHub.
    pub url: String,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Newer(Release),
    UpToDate,
    Failed(String),
}

/// Ask GitHub (one HTTPS request) and compare with `current`.
pub fn check(current: &str) -> Found {
    match crate::net::domain::http_get(LATEST, true) {
        Ok(body) => compare(&body, current),
        Err(e) => Found::Failed(e),
    }
}

/// Read the API's answer and compare its tag with `current`.
fn compare(body: &str, current: &str) -> Found {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return Found::Failed(format!("unexpected answer from GitHub: {e}")),
    };
    let Some(tag) = v.get("tag_name").and_then(|t| t.as_str()) else {
        return Found::Failed("no release found".into());
    };
    let (Some(latest), Some(now)) = (semver(tag), semver(current)) else {
        return Found::Failed(format!("can't read the version {tag:?}"));
    };
    if latest <= now {
        return Found::UpToDate;
    }
    // The link and the version shown are built from the numbers read, not
    // taken as sent: nothing in the answer reaches the system's launcher
    // or the screen beyond three numbers.
    let (a, b, c) = latest;
    Found::Newer(Release {
        version: format!("{a}.{b}.{c}"),
        url: format!("{RELEASES}/tag/v{a}.{b}.{c}"),
    })
}

/// "v1.2.3" or "1.2.3" (anything after a - or + ignored) as numbers.
fn semver(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (it.next()??, it.next()??, it.next().flatten().unwrap_or(0));
    it.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(tag: &str, url: &str) -> String {
        serde_json::json!({ "tag_name": tag, "html_url": url }).to_string()
    }

    #[test]
    fn a_newer_release_is_found_with_its_link() {
        let url = "https://github.com/hammerknife404/is-alive/releases/tag/v0.24.0";
        assert_eq!(
            compare(&body("v0.24.0", url), "0.23.0"),
            Found::Newer(Release {
                version: "0.24.0".into(),
                url: url.into()
            })
        );
        // 0.10 is newer than 0.9: numbers, not text.
        assert!(matches!(
            compare(&body("v0.10.0", url), "0.9.5"),
            Found::Newer(_)
        ));
    }

    #[test]
    fn the_same_or_older_is_up_to_date() {
        let url = "https://github.com/hammerknife404/is-alive/releases/tag/v0.23.0";
        assert_eq!(compare(&body("v0.23.0", url), "0.23.0"), Found::UpToDate);
        assert_eq!(compare(&body("0.22.1", url), "0.23.0"), Found::UpToDate);
    }

    /// The link is the project's own release page for the numbers read,
    /// whatever the answer sends: another site, or a link on the project's
    /// own with something tacked on for the launcher to read.
    #[test]
    fn the_link_is_built_not_taken() {
        let ours = Found::Newer(Release {
            version: "1.0.0".into(),
            url: format!("{RELEASES}/tag/v1.0.0"),
        });
        for sent in [
            "https://evil.example/x",
            "https://github.com/hammerknife404/is-alive/x,/root,\\\\203.0.113.5\\s\\a.exe",
        ] {
            assert_eq!(compare(&body("v1.0.0", sent), "0.23.0"), ours, "{sent}");
        }
        // A tag with more than the numbers shows only the numbers.
        assert_eq!(
            compare(&body("v1.0.0-rc1 and then some", ""), "0.23.0"),
            ours
        );
    }

    #[test]
    fn odd_answers_fail_rather_than_guess() {
        assert!(matches!(compare("not json", "0.23.0"), Found::Failed(_)));
        assert!(matches!(compare("{}", "0.23.0"), Found::Failed(_)));
        assert!(matches!(
            compare(&body("nightly", RELEASES), "0.23.0"),
            Found::Failed(_)
        ));
        assert_eq!(semver("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(semver("1.2"), Some((1, 2, 0)));
        assert_eq!(semver("1.2.3.4"), None);
    }
}
